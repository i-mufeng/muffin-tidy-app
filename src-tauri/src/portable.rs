//! 便携设备（MTP）导入：枚举设备/目录 → 复制选定项到会话临时目录 → 交给现有扫描链路。
//!
//! 由阶段 0 PoC（poc.rs）正式化而来。Windows-only，COM 工作跑在专用 STA 线程
//! （`IFileOperation` 强制 STA）。MTP 内部对象用 PIDL（hex 序列化）作为跨调用句柄
//! （parsing name 对 MTP 深层会 E_INVALIDARG）。
//!
//! 进度：复制由 `IFileOperation` 整体执行（已验证可靠），用一个监控线程轮询临时目录
//! 文件数/字节数上报，避免实现繁琐的 `IFileOperationProgressSink`。取消会在文件边界生效；统计期间也检查取消。

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
pub async fn pd_browse(parent_id: Option<String>) -> Result<Vec<PortableEntry>, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || imp::browse(parent_id))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
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
        let guard = imp::begin_import().map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || imp::run_import(item_ids, on_progress, guard))
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
    let path = std::path::PathBuf::from(temp_dir);
    let mut owned = COMPLETED_DIRS.lock().map_err(|_| "会话目录锁不可用")?;
    if !owned.contains(&path) {
        return Err("拒绝清理：不是本次应用创建且已完成的导入目录".into());
    }
    validate_session_path(&path)?;
    std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
    owned.remove(&path);
    Ok(())
}

static COMPLETED_DIRS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

fn validate_session_path(path: &std::path::Path) -> Result<(), String> {
    let base = std::env::temp_dir().join("muffin-tidy-import");
    if path.parent() != Some(base.as_path())
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("拒绝清理：必须是导入根目录的直接子目录".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || path.canonicalize().map_err(|e| e.to_string())?.parent()
            != Some(base.canonicalize().map_err(|e| e.to_string())?.as_path())
    {
        return Err("拒绝清理：目录已被替换或重定向".into());
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_pidl(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 4 || bytes.len() > 65536 {
        return Err("非法 PIDL 长度".into());
    }
    let mut offset = 0;
    while offset + 2 <= bytes.len() {
        let size = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if size == 0 {
            return if offset > 0 && offset + 2 == bytes.len() {
                Ok(())
            } else {
                Err("非法 PIDL 终止符".into())
            };
        }
        if size < 2 || size > bytes.len() - offset {
            return Err("非法 PIDL 节点长度".into());
        }
        offset += size;
    }
    Err("PIDL 缺少终止符".into())
}

#[cfg(any(windows, test))]
fn create_session_dir() -> std::io::Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join("muffin-tidy-import");
    std::fs::create_dir_all(&base)?;
    loop {
        let name = format!(
            "{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = base.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(any(windows, test))]
mod import_control {
    use anyhow::{anyhow, Result};
    // 在 IPC 入口登记任务，避免 blocking worker 启动前发出的取消被复位丢失。
    static IMPORT_STATE: std::sync::Mutex<(bool, bool)> = std::sync::Mutex::new((false, false));

    pub struct ImportGuard;
    impl Drop for ImportGuard {
        fn drop(&mut self) {
            if let Ok(mut state) = IMPORT_STATE.lock() {
                state.0 = false;
            }
        }
    }
    pub fn begin_import() -> Result<ImportGuard> {
        let mut state = IMPORT_STATE
            .lock()
            .map_err(|_| anyhow!("导入状态锁不可用"))?;
        if state.0 {
            return Err(anyhow!("已有导入正在进行"));
        }
        *state = (true, false);
        Ok(ImportGuard)
    }
    pub fn request_cancel() {
        if let Ok(mut state) = IMPORT_STATE.lock() {
            if state.0 {
                state.1 = true;
            }
        }
    }
    pub fn is_cancelled() -> bool {
        IMPORT_STATE.lock().map(|state| state.1).unwrap_or(true)
    }

    #[cfg(test)]
    #[test]
    fn cancellation_before_worker_start_survives_and_concurrent_import_is_rejected() {
        let guard = begin_import().unwrap();
        request_cancel();
        assert!(is_cancelled());
        assert!(begin_import().is_err());
        assert!(is_cancelled());
        drop(guard);
        let next = begin_import().unwrap();
        assert!(!is_cancelled());
        drop(next);
    }
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
    use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_ComputerFolder, FileOperation, IEnumShellItems, IFileOperation,
        IFileOperationProgressSink, IFileOperationProgressSink_Impl, ILFree, ILGetSize, IShellItem,
        SHCreateItemFromIDList, SHCreateItemFromParsingName, SHGetIDListFromObject,
        SHGetKnownFolderItem, FOF_NO_UI, KF_FLAG_DEFAULT, SIGDN, SIGDN_NORMALDISPLAY,
    };

    use super::import_control::is_cancelled;
    pub use super::import_control::{begin_import, request_cancel, ImportGuard};

    /// 进度回调：记录逐项失败并处理取消。`PreCopyItem` 在每个文件复制前触发，见到取消标志即返回
    /// `E_ABORT` → 据微软文档，整批 `IFileOperation` 及后续操作随之取消。其余回调一律放行。
    /// 进度上报仍由 `run_import` 的轮询监控线程负责，与本 sink 解耦。
    #[implement(IFileOperationProgressSink)]
    struct CancelSink {
        failed: Arc<std::sync::Mutex<Option<HRESULT>>>,
    }

    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for CancelSink_Impl {
        fn StartOperations(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, hrresult: HRESULT) -> windows::core::Result<()> {
            if hrresult.is_err() {
                *self.failed.lock().unwrap() = Some(hrresult);
            }
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
            if is_cancelled() {
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
            hrcopy: HRESULT,
            _psinewlycreated: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            if hrcopy.is_err() {
                *self.failed.lock().unwrap() = Some(hrcopy);
            }
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
            0x8007_0005 => {
                "无法访问设备：请确认手机已解锁，并在手机上点「信任此电脑」后重试".into()
            }
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
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
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
        if handle.len() > 131072 {
            return Err(anyhow!("句柄过长"));
        }
        let bytes = hex::decode(handle).map_err(|_| anyhow!("非法句柄"))?;
        super::validate_pidl(&bytes).map_err(anyhow::Error::msg)?;
        Ok(SHCreateItemFromIDList(bytes.as_ptr() as *const ITEMIDLIST)?)
    }

    /// 递归统计某项下的文件数（文件夹则枚举子项，文件计 1）。
    fn count_item_files(item: &IShellItem) -> Result<usize> {
        if is_cancelled() {
            return Err(anyhow!("导入已取消"));
        }
        unsafe {
            let attrs = item.GetAttributes(SFGAO_FOLDER)?;
            if (attrs & SFGAO_FOLDER).0 == 0 {
                return Ok(1);
            }
            let enumer: IEnumShellItems = match item.BindToHandler(None, &BHID_EnumItems) {
                Ok(e) => e,
                Err(e) => return Err(e.into()),
            };
            let mut n = 0;
            loop {
                let mut buf: [Option<IShellItem>; 1] = [None];
                let mut fetched = 0u32;
                enumer.Next(&mut buf, Some(&mut fetched))?;
                if fetched == 0 {
                    break;
                }
                if let Some(child) = buf[0].take() {
                    n += count_item_files(&child)?;
                }
            }
            Ok(n)
        }
    }

    /// 统计本地目录的文件数与总字节（监控进度用）。
    fn dir_stat(dir: &Path) -> (usize, u64) {
        let mut n = 0usize;
        let mut b = 0u64;
        for e in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
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
                enumer.Next(&mut buf, Some(&mut fetched))?;
                if fetched == 0 {
                    break;
                }
                let Some(item) = buf[0].take() else { break };
                let attrs = item.GetAttributes(SFGAO_FILESYSTEM | SFGAO_FOLDER)?;
                let is_filesystem = (attrs & SFGAO_FILESYSTEM).0 != 0;
                let is_folder = (attrs & SFGAO_FOLDER).0 != 0;
                // 顶层只保留便携设备：非文件系统的可展开项（过滤掉本地磁盘/库）
                if devices_only && (is_filesystem || !is_folder) {
                    continue;
                }
                let Ok(id) = item_to_handle(&item) else {
                    continue;
                };
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

    pub fn run_import(
        item_ids: Vec<String>,
        ch: Channel<ImportProgress>,
        _guard: ImportGuard,
    ) -> Result<ImportResult> {
        if item_ids.is_empty() {
            return Err(anyhow!("请选择要导入的文件或目录"));
        }
        let start = Instant::now();

        // 1) 统计总文件数（counting）——便于前端显示百分比
        let _ = ch.send(ImportProgress {
            phase: "counting".into(),
            done_files: 0,
            total_files: 0,
            done_bytes: 0,
        });
        let mut seen = std::collections::HashSet::new();
        let item_ids: Vec<_> = item_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        let ids = item_ids.clone();
        let count_result = run_sta(move || unsafe {
            let mut n = 0;
            for id in &ids {
                let it = item_from_handle(id)?;
                n += count_item_files(&it)?;
            }
            Ok(n)
        });
        if is_cancelled() {
            return Ok(ImportResult {
                temp_dir: String::new(),
                file_count: 0,
                elapsed_ms: start.elapsed().as_millis(),
                cancelled: true,
            });
        }
        let total_files = count_result?;
        if total_files == 0 {
            return Err(anyhow!("所选目录中没有可导入的文件"));
        }
        let _ = ch.send(ImportProgress {
            phase: "counting".into(),
            done_files: 0,
            total_files,
            done_bytes: 0,
        });

        // 2) 会话临时目录
        let temp_dir = super::create_session_dir()?;

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
            let dst: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(dst_str.as_str()), None)?;
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(FOF_NO_UI)?;

            // 注册取消回调：PreCopyItem 见到取消标志即返回 E_ABORT 中止整批操作
            let failed = Arc::new(std::sync::Mutex::new(None));
            let sink: IFileOperationProgressSink = CancelSink {
                failed: failed.clone(),
            }
            .into();
            let cookie = op.Advise(&sink)?;

            for id in &ids {
                let src = item_from_handle(id)?;
                op.CopyItem(&src, &dst, PCWSTR::null(), None)?;
            }
            let perform = op.PerformOperations();
            let _ = op.Unadvise(cookie);

            // 用户取消优先判定：取消状态只由 pd_cancel_import 置位，是唯一的取消来源。
            // 取消后 PerformOperations 可能返回 E_ABORT 失败、也可能仍返回成功码（见微软文档），
            // 故先看本标志而非 HRESULT，避免把取消当失败、或把磁盘满等系统中止误当取消。
            if is_cancelled() {
                return Ok(true);
            }
            if let Some(hr) = *failed.lock().unwrap() {
                return Err(anyhow!(friendly_copy_error(
                    &windows::core::Error::from_hresult(hr)
                )));
            }
            if op.GetAnyOperationsAborted()?.as_bool() {
                return Err(anyhow!("设备复制未完成，请检查连接和磁盘空间"));
            }
            match perform {
                Ok(()) => Ok(false),
                Err(e) => Err(anyhow!(friendly_copy_error(&e))),
            }
        });

        // 5) 停止监控（无论成败），再处理复制结果
        stop.store(true, Ordering::Relaxed);
        let _ = monitor.join();
        let cancelled = match copy_res {
            Ok(value) => value,
            Err(error) => {
                if let Err(cleanup) = std::fs::remove_dir_all(&temp_dir) {
                    return Err(anyhow!(
                        "{error}；临时目录清理失败 {}：{cleanup}",
                        temp_dir.display()
                    ));
                }
                return Err(error);
            }
        };

        if cancelled {
            // 用户取消：清理半成品临时目录，静默返回（前端据 cancelled 回到浏览态）
            std::fs::remove_dir_all(&temp_dir)
                .map_err(|e| anyhow!("取消后清理失败 {}：{e}", temp_dir.display()))?;
            return Ok(ImportResult {
                temp_dir: String::new(),
                file_count: 0,
                elapsed_ms: start.elapsed().as_millis(),
                cancelled: true,
            });
        }

        let (final_n, final_b) = dir_stat(&temp_dir);
        if final_n < total_files {
            std::fs::remove_dir_all(&temp_dir)?;
            return Err(anyhow!(
                "导入不完整：预期 {total_files} 个文件，实际复制 {final_n} 个，请重新选择后重试"
            ));
        }
        super::COMPLETED_DIRS
            .lock()
            .map_err(|_| anyhow!("会话目录锁不可用"))?
            .insert(temp_dir.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pidl_rejects_truncated_or_malformed_nodes() {
        for bytes in [
            vec![],
            vec![0, 0],
            vec![1, 0, 0, 0],
            vec![8, 0, 0, 0],
            vec![2, 0],
            vec![2, 0, 0, 0, 9],
        ] {
            assert!(validate_pidl(&bytes).is_err(), "{bytes:?}");
        }
        assert!(validate_pidl(&[4, 0, 7, 8, 2, 0, 0, 0]).is_ok());
    }
    #[test]
    fn sessions_are_unique_and_cleanup_requires_ownership() {
        let first = create_session_dir().unwrap();
        let second = create_session_dir().unwrap();
        assert_ne!(first, second);
        assert!(pd_cleanup(first.to_string_lossy().into()).is_err());
        COMPLETED_DIRS.lock().unwrap().insert(first.clone());
        assert!(pd_cleanup(first.to_string_lossy().into()).is_ok());
        assert!(second.exists());
        std::fs::remove_dir(second).unwrap();
    }
    #[test]
    fn cleanup_rejects_root_and_traversal() {
        let root = std::env::temp_dir().join("muffin-tidy-import");
        assert!(validate_session_path(&root).is_err());
        assert!(validate_session_path(&root.join("session/../outside")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_replaced_directory_symlink() {
        let session = create_session_dir().unwrap();
        let other = create_session_dir().unwrap();
        std::fs::write(other.join("keep"), "original").unwrap();
        COMPLETED_DIRS.lock().unwrap().insert(session.clone());
        std::fs::remove_dir(&session).unwrap();
        std::os::unix::fs::symlink(&other, &session).unwrap();
        assert!(pd_cleanup(session.to_string_lossy().into()).is_err());
        assert!(other.join("keep").exists());
        COMPLETED_DIRS.lock().unwrap().remove(&session);
        std::fs::remove_file(session).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }
}
