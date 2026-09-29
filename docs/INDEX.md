# 文档索引

> 最后更新：2026-06-25

## 目录结构

- [design/](./design/) - 设计文档（架构、方案、流程图）
- [plans/](./plans/) - 计划文档（实施计划、里程碑）
- [tasks/](./tasks/) - 任务文档（任务分配、进度跟踪）
- [testing/](./testing/) - 测试文档（测试用例、测试报告）
- [guides/](./guides/) - 使用指南（开发/部署）

## 最近更新

| 日期 | 文档 | 说明 |
|------|------|------|
| 2026-06-25 | [手机直连导入（MTP）详细设计](./design/2026-06-21-phone-direct-import-design.md) | 订正：§12.2 根因更正为 opener path 作用域为空集（拒一切路径，非「仅目录」）；新增 Rust 命令 `open_path_default` 修复「打开日志」按钮，去掉静默 catch，移除死权限 `opener:allow-open-path` |
| 2026-06-25 | [手机直连导入（MTP）详细设计](./design/2026-06-21-phone-direct-import-design.md) | 更新：移除 PoC（poc.rs / PoCDevicePanel.vue 及 lib.rs/HomeView.vue 全部引用），正式入口为其超集 |
| 2026-06-23 | [手机直连导入（MTP）详细设计](./design/2026-06-21-phone-direct-import-design.md) | 更新：§12 阶段 1 落地记录（取消/openPath 修复/错误细化/临时清理；PoC 移除暂缓） |
| 2026-06-21 | [手机直连导入（MTP）详细设计](./design/2026-06-21-phone-direct-import-design.md) | 新增：方案 B（Shell COM 枚举 + 复制到临时目录再扫描）详细设计与任务拆解 |
