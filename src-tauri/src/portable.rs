//! 便携设备（MTP）导入：枚举设备/目录 → 复制选定项到会话临时目录 → 交给现有扫描链路。
//!
//! 由阶段 0 PoC（poc.rs）正式化而来。Windows-only，COM 工作跑在专用 STA 线程
//! （`IFileOperation` 强制 STA）。MTP 内部对象用 PIDL（hex 序列化）作为跨调用句柄
//! （parsing name 对 MTP 深层会 E_INVALIDARG）。
//!
//! 进度：复制由 `IFileOperation` 整体执行（已验证可靠），用一个监控线程轮询临时目录
//! 文件数/字节数上报，避免实现繁琐的 `IFileOperationProgressSink`。中途取消见后续迭代。

use serde::Serialize;
use tauri::ipc::Channel;

#[derive(Debug, Serialize)]
pub struct PortableEntry {
    pub name: String,
    /// PIDL 的 hex 序列化句柄
    pub id: String,
    pub is_folder: bool,
    pub is_filesystem: bool,
}

#[derive(Clone, Serialize)]
pub struct ImportProgress {
    /// "counting" | "copying" | "done"
    pub phase: String,
    pub done_files: usize,
    pub total_files: usize,
    pub done_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub temp_dir: String,
    pub file_count: usize,
    pub elapsed_ms: u128,
    /// true = 用户中途取消（临时目录已清理，前端应静默回到浏览态）
    pub cancelled: bool,
}

/// 浏览：`parent_id = None` 仅返回便携设备（过滤本地磁盘）；`Some` 返回该节点全部子项。
#[tauri::command]
pub fn pd_browse(parent_id: Option<String>) -> Result<Vec<PortableEntry>, String> {
    #[cfg(windows)]
    {
        imp::browse(parent_id).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = parent_id;
        Err("便携设备访问仅支持 Windows".into())
    }
}

/// 复制选定项（PIDL hex 句柄）到会话临时目录，进度经 Channel 上报。返回临时目录路径。
#[tauri::command]
pub async fn pd_import(
    item_ids: Vec<String>,
    on_progress: Channel<ImportProgress>,
) -> Result<ImportResult, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || imp::run_import(item_ids, on_progress))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (item_ids, on_progress);
        Err("便携设备访问仅支持 Windows".into())
    }
}

/// 请求取消正在进行的导入。透传给复制线程的进度回调（`PreCopyItem` 见到标志即返回
/// `E_ABORT`，在下一个文件边界中止整批 `IFileOperation`）。空操作时调用也安全。
#[tauri::command]
pub fn pd_cancel_import() {
    #[cfg(windows)]
    imp::request_cancel();
}

/// 清理某次导入的会话临时目录。仅允许删除 `%TEMP%/muffin-tidy-import` 下的目录。
#[tauri::command]
pub fn pd_cleanup(temp_dir: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(&temp_dir);
    let base = std::env::temp_dir().join("muffin-tidy-import");
    if !path.starts_with(&base) {
        return Err("拒绝清理：路径不在导入临时目录内".into());
    }
    std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
}

#[cfg(windows)]
mod imp {
    use super::{ImportProgress, ImportResult, PortableEntry};
    use anyhow::{anyhow, Result};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tauri::ipc::Channel;
    use windows::core::{implement, Ref, HRESULT, HSTRING, PCWSTR};
    use windows::Win32::Foundation::E_ABORT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FLAGS, SFGAO_FOLDER};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FileOperation, IEnumShellItems, IFileOperation,
        IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem, ILFree, ILGetSize,
        SHCreateItemFromIDList, SHCreateItemFromParsingName, SHGetIDListFromObject,
        SHGetKnownFolderItem, FOF_NO_UI, FOLDERID_ComputerFolder, KF_FLAG_DEFAULT, SIGDN,
        SIGDN_NORMALDISPLAY,
    };

    /// 全局取消标志：导入开始时复位，`pd_cancel_import` 置位，复制进度回调读取。
    /// UI 保证同一时刻仅一次导入在跑，故单个全局标志足矣。
    static IMPORT_CANCEL: AtomicBool = AtomicBool::new(false);

    pub fn request_cancel() {
        IMPORT_CANCEL.store(true, Ordering::Relaxed);
    }

    /// 进度回调：仅负责「取消」。`PreCopyItem` 在每个文件复制前触发，见到取消标志即返回
    /// `E_ABORT` → 据微软文档，整批 `IFileOperation` 及后续操作随之取消。其余回调一律放行。
    /// 进度上报仍由 `run_import` 的轮询监控线程负责，与本 sink 解耦。
    #[implement(IFileOperationProgressSink)]
    struct CancelSink;

    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for CancelSink_Impl {
        fn StartOperations(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, _hrresult: HRESULT) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreRenameItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostRenameItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
            _hrrename: HRESULT,
            _psinewlycreated: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreMoveItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostMoveItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
            _hrmove: HRESULT,
            _psinewlycreated: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreCopyItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
        ) -> windows::core::Result<()> {
            if IMPORT_CANCEL.load(Ordering::Relaxed) {
                Err(E_ABORT.into())
            } else {
                Ok(())
            }
        }
        fn PostCopyItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
            _hrcopy: HRESULT,
            _psinewlycreated: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreDeleteItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _dwflags: u32,
            _psiitem: Ref<'_, IShellItem>,
            _hrdelete: HRESULT,
            _psinewlycreated: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreNewItem(
            &self,
            _dwflags: u32,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostNewItem(
            &self,
            _dwflags: u32,
            _psidestinationfolder: Ref<'_, IShellItem>,
            _psznewname: &PCWSTR,
            _psztemplatename: &PCWSTR,
            _dwfileattributes: u32,
            _hrnew: HRESULT,
            _psinewitem: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
    }

    /// 把复制失败的 HRESULT 映射成对用户友好的中文提示。
    fn friendly_copy_error(e: &windows::core::Error) -> String {
        let code = e.code().0 as u32;
        match code {
            // ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL
            0x8007_0070 | 0x8007_0027 => {
                "磁盘空间不足：临时目录所在磁盘剩余空间不够，请清理后重试".into()
            }
            // E_ACCESSDENIED：常见于手机锁屏 / 未点「信任此电脑」
            0x8007_0005 => "无法访问设备：请确认手机已解锁，并在手机上点「信任此电脑」后重试".into(),
            // ERROR_DEVICE_NOT_CONNECTED / 设备掉线
            0x8007_048F | 0x8007_001F => "设备读取失败：请检查 USB 连接是否稳定后重试".into(),
            _ => format!("复制失败（0x{:08X}）：{}", code, e.message()),
        }
    }

    /// 在专用 STA 线程上执行 COM 闭包：IFileOperation 强制要求 STA。
    fn run_sta<T, F>(f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let r = f();
            CoUninitialize();
            r
        })
        .join()
        .map_err(|_| anyhow!("STA 线程崩溃"))?
    }

    unsafe fn display_name(item: &IShellItem, kind: SIGDN) -> String {
        match item.GetDisplayName(kind) {
            Ok(pw) => {
                let s = pw.to_string().unwrap_or_default();
                CoTaskMemFree(Some(pw.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }

    unsafe fn item_to_handle(item: &IShellItem) -> Result<String> {
        let pidl = SHGetIDListFromObject(item)?;
        let size = ILGetSize(Some(pidl)) as usize;
        let hex = if size > 0 {
            hex::encode(std::slice::from_raw_parts(pidl as *const u8, size))
        } else {
            String::new()
        };
        ILFree(Some(pidl));
        if hex.is_empty() {
            return Err(anyhow!("空 PIDL"));
        }
        Ok(hex)
    }

    unsafe fn item_from_handle(handle: &str) -> Result<IShellItem> {
        let bytes = hex::decode(handle).map_err(|_| anyhow!("非法句柄"))?;
        Ok(SHCreateItemFromIDList(bytes.as_ptr() as *const ITEMIDLIST)?)
    }

    /// 递归统计某项下的文件数（文件夹则枚举子项，文件计 1）。
    fn count_item_files(item: &IShellItem) -> usize {
        unsafe {
            let attrs = item
                .GetAttributes(SFGAO_FOLDER)
                .unwrap_or(SFGAO_FLAGS(0));
            if (attrs & SFGAO_FOLDER).0 == 0 {
                return 1;
            }
            let enumer: IEnumShellItems = match item.BindToHandler(None, &BHID_EnumItems) {
                Ok(e) => e,
                Err(_) => return 0,
            };
            let mut n = 0;
            loop {
                let mut buf: [Option<IShellItem>; 1] = [None];
                let mut fetched = 0u32;
                if enumer.Next(&mut buf, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if let Some(child) = buf[0].take() {
                    n += count_item_files(&child);
                }
            }
            n
        }
    }

    /// 统计本地目录的文件数与总字节（监控进度用）。
    fn dir_stat(dir: &Path) -> (usize, u64) {
        let mut n = 0usize;
        let mut b = 0u64;
        for e in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if e.file_type().is_file() {
                n += 1;
                b += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        (n, b)
    }

    pub fn browse(parent_id: Option<String>) -> Result<Vec<PortableEntry>> {
        run_sta(move || unsafe {
            let (parent, devices_only): (IShellItem, bool) = match &parent_id {
                None => (
                    SHGetKnownFolderItem(&FOLDERID_ComputerFolder, KF_FLAG_DEFAULT, None)?,
                    true,
                ),
                Some(handle) => (item_from_handle(handle)?, false),
            };

            let enumer: IEnumShellItems = parent.BindToHandler(None, &BHID_EnumItems)?;
            let mut out = Vec::new();
            loop {
                let mut buf: [Option<IShellItem>; 1] = [None];
                let mut fetched = 0u32;
                if enumer.Next(&mut buf, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(item) = buf[0].take() else { break };
                let attrs = item
                    .GetAttributes(SFGAO_FILESYSTEM | SFGAO_FOLDER)
                    .unwrap_or(SFGAO_FLAGS(0));
                let is_filesystem = (attrs & SFGAO_FILESYSTEM).0 != 0;
                let is_folder = (attrs & SFGAO_FOLDER).0 != 0;
                // 顶层只保留便携设备：非文件系统的可展开项（过滤掉本地磁盘/库）
                if devices_only && (is_filesystem || !is_folder) {
                    continue;
                }
                let Ok(id) = item_to_handle(&item) else { continue };
                out.push(PortableEntry {
                    name: display_name(&item, SIGDN_NORMALDISPLAY),
                    id,
                    is_folder,
                    is_filesystem,
                });
            }
            Ok(out)
        })
    }

    pub fn run_import(item_ids: Vec<String>, ch: Channel<ImportProgress>) -> Result<ImportResult> {
        let start = Instant::now();
        IMPORT_CANCEL.store(false, Ordering::Relaxed); // 复位上一次导入可能遗留的取消标志

        // 1) 统计总文件数（counting）——便于前端显示百分比
        let ids = item_ids.clone();
        let total_files = run_sta(move || unsafe {
            let mut n = 0;
            for id in &ids {
                if let Ok(it) = item_from_handle(id) {
                    n += count_item_files(&it);
                }
            }
            Ok(n)
        })?;
        let _ = ch.send(ImportProgress {
            phase: "counting".into(),
            done_files: 0,
            total_files,
            done_bytes: 0,
        });

        // 2) 会话临时目录
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let temp_dir = std::env::temp_dir().join("muffin-tidy-import").join(&stamp);
        std::fs::create_dir_all(&temp_dir)?;

        // 3) 监控线程：轮询临时目录已落地的文件数/字节数 → 进度
        let stop = Arc::new(AtomicBool::new(false));
        let monitor = {
            let stop = stop.clone();
            let ch = ch.clone();
            let dir = temp_dir.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let (n, b) = dir_stat(&dir);
                    let _ = ch.send(ImportProgress {
                        phase: "copying".into(),
                        done_files: n,
                        total_files,
                        done_bytes: b,
                    });
                    std::thread::sleep(Duration::from_millis(200));
                }
            })
        };

        // 4) STA 线程执行复制（IFileOperation，已验证可靠）。挂载取消回调；
        //    闭包返回 Ok(true)=被取消 / Ok(false)=正常完成 / Err=真实失败（已映射为友好提示）。
        let ids = item_ids;
        let dst_str = temp_dir.to_string_lossy().to_string();
        let copy_res: Result<bool> = run_sta(move || unsafe {
            let dst: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dst_str.as_str()), None)?;
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(FOF_NO_UI)?;

            // 注册取消回调：PreCopyItem 见到取消标志即返回 E_ABORT 中止整批操作
            let sink: IFileOperationProgressSink = CancelSink.into();
            let cookie = op.Advise(&sink)?;

            for id in &ids {
                let src = item_from_handle(id)?;
                op.CopyItem(&src, &dst, PCWSTR::null(), None)?;
            }
            let perform = op.PerformOperations();
            let _ = op.Unadvise(cookie);

            // 用户取消优先判定：IMPORT_CANCEL 只由 pd_cancel_import 置位，是唯一的取消来源。
            // 取消后 PerformOperations 可能返回 E_ABORT 失败、也可能仍返回成功码（见微软文档），
            // 故先看本标志而非 HRESULT，避免把取消当失败、或把磁盘满等系统中止误当取消。
            if IMPORT_CANCEL.load(Ordering::Relaxed) {
                return Ok(true);
            }
            match perform {
                Ok(()) => Ok(false),
                Err(e) => Err(anyhow!(friendly_copy_error(&e))),
            }
        });

        // 5) 停止监控（无论成败），再处理复制结果
        stop.store(true, Ordering::Relaxed);
        let _ = monitor.join();
        let cancelled = copy_res?;

        if cancelled {
            // 用户取消：清理半成品临时目录，静默返回（前端据 cancelled 回到浏览态）
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Ok(ImportResult {
                temp_dir: String::new(),
                file_count: 0,
                elapsed_ms: start.elapsed().as_millis(),
                cancelled: true,
            });
        }

        let (final_n, final_b) = dir_stat(&temp_dir);
        let _ = ch.send(ImportProgress {
            phase: "done".into(),
            done_files: final_n,
            total_files,
            done_bytes: final_b,
        });

        Ok(ImportResult {
            temp_dir: temp_dir.to_string_lossy().to_string(),
            file_count: final_n,
            elapsed_ms: start.elapsed().as_millis(),
            cancelled: false,
        })
    }
}
