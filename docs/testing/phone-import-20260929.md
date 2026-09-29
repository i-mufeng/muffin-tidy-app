# 手机直连导入修复与验证（2026-09-29）

## 来源与整合方式

来源：`origin/main` 的 `8e9d124 手机直连导入`。通过 `git fetch origin` 获取，再将该提交相对本地 HEAD 的差异以 `git apply --check` / `git apply` 导入当前检出；保留原有扫描背压相关未提交修改。随后为推送在 `origin/main` 上建立独立工作树，仅提取手机直连修复；扫描背压等原有未提交工作保留在原检出，未纳入本次提交。

## 修复范围

- 补齐首页入口、App 弹窗挂载、Store 导入状态和扫描接线；修复缺少 `importOpen` / `closeImport` / `openImportedDirectory` 的 TypeScript 错误。
- 添加原生平台探测；非 Windows 显示可操作的本地目录替代方案。
- 浏览关闭/重开后丢弃旧响应，导入期间阻止重复操作；取消失败可见并可重试。
- 导入弹窗支持焦点约束、Tab/Shift+Tab、Escape 和关闭后焦点恢复；背景不可操作。
- Windows 依赖补充 Shell Common、SystemServices 和 COM 实现宏所需 `windows-core`。
- 设备浏览移出窗口线程；校验 PIDL 字节结构；传播枚举和逐文件复制错误，检查操作中止及复制数量，避免失败误报成功。
- 会话目录独占创建；清理需要本进程完成会话的所有权与路径校验。
- 移除启动时删除整个缓存根目录；成功导入缓存不会因重置工作集被删除，避免跨实例和未导出数据损失。

## 当前功能边界

仅 Windows USB/MTP。当前选设备后复制其存储节点内容，再由扫描器筛选媒体，尚无按相册选择或仅复制媒体的优化。大存储会占用较多临时空间；取消在枚举检查点或文件复制边界生效，不能保证立即打断 Windows 正在处理的单个文件。

成功导入缓存保留在系统临时目录 `muffin-tidy-import` 下；应通过现有导出操作保存到用户选定目录。暂未提供已完成缓存的用户清理界面，不自动删除其他运行实例或过去会话的缓存。

## 验证

以下数量来自提取后的独立提交候选；不包含原共享检出中的扫描背压测试。

- `bun run build`：通过 Vue 类型检查与 Vite 生产构建。
- `bun test tests/project-scan.test.ts tests/media-queue.test.ts`：23 pass / 0 fail，包括新增导入扫描、失败、取消、忙碌和空路径回归。
- Windows 全应用交叉检查受本机资源编译器缺失阻断：`NotAttempted("llvm-rc")`。不等同于 Windows 打包成功。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：28 passed / 0 failed / 1 ignored；跳过原有 20,000 文件与 100 GiB 稀疏文件压力用例。新增 5 项覆盖取消启动竞态、并发隔离、PIDL 与会话清理。
- `portable.rs` Windows 目标类型检查：通过。使用临时最小 crate 的 `#[path]` 直接引用工作区源文件，绕过整应用资源构建脚本；只代表该模块的 Windows API 类型检查。
- `tests/device-import.browser.mjs`：浏览器 IPC 模拟回归通过，覆盖平台、异步过期响应、焦点、取消失败与重试、取消恢复、进度、平台探测失败后刷新恢复、成功导入临时目录后进入扫描及工作区、桌面及 390/320 像素宽度。此项不连接真实设备。

未进行 Windows 真机手机连接、信任授权、断线、磁盘满及大量文件传输验收。

Windows 模块校验命令（临时 crate 保留于本机临时目录）：

```bash
CARGO_TARGET_DIR=/Users/mufeng/Code/Githb/muffin-tidy-app/src-tauri/target cargo check --offline --manifest-path /var/folders/3s/35wj2l453l53yz7nbkylzpbh0000gn/T/tidy-portable-check-ah6e9hzv/Cargo.toml --target x86_64-pc-windows-msvc
```

浏览器回归复现（先启动 Vite，再另开终端运行；本机捆绑 Playwright 路径）：

```bash
bun run dev --host 127.0.0.1 --port 1420
```

```bash
TIDY_URL=http://127.0.0.1:1420 PLAYWRIGHT_MODULE=/Users/mufeng/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/index.mjs node tests/device-import.browser.mjs
```

2026-09-29 本地最终输出：`PASS: browser IPC mocks ... successful import-to-scan-to-workspace ... no page errors.` 成功场景断言扫描路径等于导入返回的临时目录，再由模拟空媒体结果进入工作区。桌面和 390px 截图写入 `os.tmpdir()` 的 `tidy-phone-desktop.png` / `tidy-phone-narrow.png`，已查看；320px 检查弹窗边界无横向溢出。
