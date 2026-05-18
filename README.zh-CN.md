# HelionCoder Desktop

HelionCoder 的本地优先桌面端，基于 [Tauri](https://tauri.app/)、Rust 和 [Svelte](https://svelte.dev/) 构建。

桌面端将 HelionCoder CLI 封装为可视化工作区，围绕三类核心场景组织：

- **Chat 聊天** — 纯 AI 对话。
- **Cowork 办公** — 在选定文件夹内直接读取、编辑、创建文件，支持 Word、Excel、PPT、PDF 等文档技能。
- **Code 编程** — 运行 HelionCoder CLI，会话包含项目文件夹、Diff、工具调用、模型/思考控制和本地用量统计。

## 功能特性

- 集成 HelionCoder CLI（默认检测 `helion-coder`，兼容 `helioncoder`）。
- 系统原生文件夹选择器，选择本地项目。
- 中英文界面（i18n）。
- 设置、历史和统计存储在 `~/.helioncoder` 本地目录。
- SQLite 统计：sessions、messages、tokens、使用模型、活跃天数、连续天数和代码增删行。
- 侧边栏提供 Skills、Plugins、MCP、Hooks、Agents、查看更改等入口。
- 基于 CodeMirror 的代码编辑器，支持 15+ 种语言语法高亮。
- 集成终端（xterm.js）。

## 平台支持

| 平台   | 状态         |
| ------ | ------------ |
| macOS  | 支持（13.0+）|
| Linux  | 支持（deb）  |
| Windows| 支持         |

## 安装说明

### macOS："已损坏，无法打开"提示

由于应用未使用 Apple 开发者证书签名，macOS Gatekeeper 可能会显示：

> "HelionCoder" 已损坏，无法打开。您应该将它移到废纸篓。

这是一个误报。请在终端中运行以下命令修复：

```bash
xattr -cr /Applications/HelionCoder.app
```

或者在首次打开时右键点击应用，选择 **打开**，macOS 会记住该例外。

## 开发

### 前置条件

- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/) 1.75+ 与 Cargo
- [Tauri CLI](https://tauri.app/)（通过 npm 安装）

### 安装

```bash
git clone https://github.com/gabrielpondc/HelionCoder-APP.git
cd HelionCoder-APP
npm install
```

### 开发运行

```bash
npm run tauri dev
```

### 编译

```bash
npm run tauri build
```

构建产物输出到 `src-tauri/target/release/bundle`。

### GitHub Actions 构建

项目包含 GitHub Actions 工作流（`.github/workflows/desktop-build.yml`），支持全平台编译。可通过以下方式手动触发：

1. 在 GitHub 上打开仓库页面。
2. 进入 **Actions** > **Build desktop app**。
3. 点击 **Run workflow**，选择分支后运行。

工作流支持以下构建目标：

| 目标              | 平台                |
| ----------------- | ------------------- |
| `macos-arm64`     | macOS Apple Silicon |
| `macos-x64`       | macOS Intel         |
| `linux`           | Ubuntu 22.04        |
| `windows`         | Windows             |

构建完成后可在工作流运行页面下载产物。

### 其他命令

```bash
npm run lint          # ESLint 检查
npm run format        # Prettier 格式化
npm run check         # svelte-check 类型检查
npm run test          # Vitest 单元测试
npm run verify        # 完整检查（lint + format + test + build + rust）
npm run fix           # 自动修复 lint + 格式化
```

## 项目结构

```
├── src/                 # Svelte 前端
├── src-tauri/           # Rust / Tauri 后端
├── messages/            # i18n 语言文件（en.json, zh-CN.json）
├── scripts/             # 构建和发布脚本
├── static/              # 静态资源
└── icon/                # 应用图标源文件
```

## 参与贡献

参见 [CONTRIBUTING.md](CONTRIBUTING.md) 了解 Bug 报告、功能请求、代码贡献和翻译指南。

## 安全

请私下报告安全漏洞，详见 [SECURITY.md](SECURITY.md)。

## 许可证

[Apache License 2.0](LICENSE)
