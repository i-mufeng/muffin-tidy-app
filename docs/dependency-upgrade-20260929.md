# 依赖升级记录（2026-09-29）

本次升级前端、Rust、Tauri及构建工作流依赖，应用版本保留 0.2.2。直接依赖以 npm/crates.io 当日最新稳定版本为目标；间接依赖刷新并受上游兼容约束限制，不强制跨越上游规定的版本范围。

## 前端

| 依赖 | 原声明 | 新声明 |
| --- | --- | --- |
| @tauri-apps/api | ^2 | ^2.12.0 |
| @tauri-apps/plugin-dialog | ^2.7.1 | ^2.8.0 |
| @tauri-apps/plugin-opener | ^2 | ^2.6.0 |
| @vue/devtools-api | 新增 | ^8.2.1 |
| @vueuse/core | ^14.3.0 | ^15.0.0 |
| pinia | ^3.0.4 | ^4.0.3 |
| virtua | ^0.49.1 | ^0.52.9 |
| vue | ^3.5.13 | ^3.5.43 |
| @tailwindcss/vite | ^4.3.1 | ^4.3.3 |
| @tauri-apps/cli | ^2 | ^2.12.0 |
| @vitejs/plugin-vue | ^5.2.1 | ^6.0.9 |
| tailwindcss | ^4.3.1 | ^4.3.3 |
| typescript | ~5.6.2 | ~6.0.3 |
| vite | ^6.0.3 | ^8.3.1 |
| vue-tsc | ^2.1.10 | ^3.3.11 |

唯一兼容例外：TypeScript 最新稳定版为 7.0.2，但与最新 vue-tsc 3.3.11 实测不兼容：

```text
Error [ERR_PACKAGE_PATH_NOT_EXPORTED]:
Package subpath './lib/tsc' is not defined by "exports"
```

采用最新兼容的 6.x 版本 `~6.0.3`，保留 Vue 类型检查。`bun outdated` 只剩这一项。Pinia 4 所需 peer `@vue/devtools-api` 已显式添加。Vite 8 使用 Rolldown/Oxc，最低 Node 为 20.19+ / 22.12+，默认浏览器目标含 Edge 111、Safari 16.4。

## Rust

| 依赖 | 原锁定版本（含间接版本） | 新直接版本 |
| --- | --- | --- |
| tauri-build | 2.6.2 | 2.7.0 |
| tauri | 2.11.2 | 2.12.0 |
| tauri-plugin-opener | 2.5.4 | 2.6.0 |
| tauri-plugin-dialog | 2.7.1 | 2.8.0 |
| tauri-plugin-fs | 2.5.1 | 2.6.0 |
| serde | 1.0.228 | 1.0.229 |
| serde_json | 1.0.150 | 1.0.151 |
| anyhow | 1.0.102 | 1.0.104 |
| walkdir | 2.5.0 | 2.5.0 |
| chrono | 0.4.45 | 0.4.45 |
| image | 0.25.10 | 0.25.10 |
| jpeg-decoder | 0.3.2 | 0.3.2 |
| rayon | 1.12.0 | 1.12.0 |
| sha2 | 0.10.9 | 0.11.0 |
| hex | 0.4.3 | 0.4.3 |
| nom-exif | 2.8.0 | 3.8.0 |
| dunce | 1.0.5 | 1.0.5 |
| once_cell | 1.21.4 | 1.21.4 |
| urlencoding | 2.1.3 | 2.1.3 |
| base64 | 0.21.7, 0.22.1 | 0.23.1 |
| filetime | 0.2.29 | 0.2.29 |
| windows | 0.61.3 | 0.62.2 |

22 个直接依赖均达到当日最新稳定版；已有最新版本的依赖补齐完整版本下限。Cargo 更新过程中刷新了 204 个包。

`nom-exif` 3.8 存在破坏性 API 变化，已迁移扫描器的来源打开、EXIF/视频解析、标签、字段值和日期访问；EXIF 日期、品牌、配对标识回归测试通过。

Tauri Rust/API/CLI 均为 2.12.0，dialog 2.8.0 与 opener 2.6.0 的 JS/Rust 对齐。Tauri 2.12 要求 Rust 1.90+，不再支持 Windows 7；本项目发布说明原本面向 Windows 10/11。该版本也包含 asset:// 异步加载修复，与慢路径导致窗口冻结的问题相关。

## 构建工作流

- actions/checkout：v5 → v7.0.1。
- oven-sh/setup-bun：v2 → v2.2.0。
- swatinem/rust-cache：v2 → v2.9.2。
- Rust stable、Bun latest 保持官方稳定渠道。
- 安装使用 `bun install --frozen-lockfile`，Rust 构建传入 `--locked`；Windows 编译前新增前端和 Rust 回归测试。
- 手动 workflow_dispatch 只验证构建；发布 Release 仍仅由版本 tag 触发。

## 本机验证

环境：Node 24.20.0、Bun 1.3.14、Rust/Cargo 1.98.1，macOS。

- 现有目录及全新临时目录 frozen 安装、Vue 类型检查、Vite 生产构建通过。
- 前端行为测试：19 通过，118 断言。
- Rust 全工程检查通过；23 项常规测试通过，1 项压力测试默认忽略并已单独执行通过。
- 单独压力样本：20,001 文件、稀疏逻辑大小 100GiB，元数据扫描约 587ms。不是实际照片解码或100GiB吞吐测试。
- 两个 Chromium IPC mock 回归通过，包括进度、取消、失败重试和两万条列表/有限预热。
- Windows 独立缩略图模块交叉类型检查通过，覆盖 windows 0.62 的 WIC/Shell 调用；这不替代 Windows 原生 UI 与真实图库验收。

## 官方依据

- [Tauri 依赖升级与 JS/Rust 对齐](https://v2.tauri.app/develop/updating-dependencies/)
- [Tauri 2.12 变更](https://v2.tauri.app/release/tauri/v2.12.0/)
- [Vite 8 迁移](https://vite.dev/guide/migration)
- [Pinia 4 变更](https://github.com/vuejs/pinia/blob/v4/packages/pinia/CHANGELOG.md)
- [vue-tsc 对 TypeScript 编译器入口的使用](https://github.com/vuejs/language-tools/blob/master/packages/tsc/index.ts)
