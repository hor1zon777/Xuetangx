# 学堂在线助手（xuetang-helper）

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri 2](https://img.shields.io/badge/Tauri-v2-24C8DB.svg)](https://tauri.app)
[![Release](https://img.shields.io/github/v/release/hor1zon777/Xuetangx?display_name=tag)](https://github.com/hor1zon777/Xuetangx/releases)

> Rust + Tauri v2 + React 实现的桌面端学堂在线自动学习工具。

## 功能

- 微信扫码登录（基于学堂在线官方 `wss://www.xuetangx.com/wsapp/`）
- 账号本地缓存与一键切换；切换 / 移除账号时自动终止旧账号正在跑的视频任务并重新加载各 tab 数据
- 课程 / 章节 / 节点遍历（视频、习题、讨论、图文）
- **自动观看视频**（按官方心跳协议 `/video-log/heartbeat/`）
  - 支持倍速、起始位置续播、停止
  - 多任务受 `task_concurrency` 限制并发，超出部分按章节顺序 FIFO 排队
  - 批量启动时按 `kg_learn_chapter` 的 DFS 顺序串行提交，避免出现"靠前章节反而排队"
- **讨论评论**（三种任务类型 tab 切换）
  - **视频讨论**：在视频节点底下的讨论区（`topic_type=0`）批量发同一条评论
  - **讨论（带分加）**：针对 `leaf_type=4` 的独立讨论节点（`topic_type=4`），发完评论后追加 `POST chapter/schedule` 触发分数累加
  - **图文**：`leaf_type=3` 节点，按章节批量标记完成（`POST chapter/schedule`）
  - 评论 429 限速自适应回退，按服务端 `Expected available in N seconds.` 等待并重试
  - 讨论列表显示从 `content.text` 抽取的案例预览，避免同名"案例分析"节点张冠李戴
- **自动作业**：拉取题目 → 询问 OpenAI 兼容大模型 → 提交答案 → 展示得分
  - 按题型（单选/多选/判断/填空/主观）下发针对性的提示词
  - 已批改的小题自动跳过
  - 完成态显式提示，避免和"运行中"混淆
- **本地题库**：把学堂在线返回的标准答案沉淀到本地，下次同题直接命中、跳过 AI
  - 答案来源仅为学堂在线已批改小题里下发的 `answer` 字段（绝对可信），AI 答出的内容不写入题库
  - 双索引命中：优先按 `problem_id` 精确匹配，回退按题面 + 选项的 `body_hash` 匹配（选项题还会校验选项 key 一致）
  - 自动作业过程中命中题库直接组装答案提交，跳过 AI 调用；批次结束默认自动收录刚被学堂批改的标准答案
  - 单节点 / 批量「收录答案」按钮：仅对已完成的节点可点击，仅 GET 不提交、完全合规
  - 独立「题库」页：统计、搜索、详情、删除、JSON 导入导出；写库后通过 `bank://updated` 事件自动刷新
- **设置面板**：OpenAI 兼容接口（base_url / api_key / model）、AI 单次请求超时、心跳间隔、默认倍速、默认评论文案、并发上限、本地题库优先 / 自动收录开关

> 本工具仅供学习交流，使用者自行承担违反平台规则的风险。

## 环境要求

- Node.js ≥ 20 + pnpm ≥ 9
- Rust 工具链（stable，≥ 1.77）
- Windows 10/11、macOS 12+、或主流 Linux 发行版
- Windows 用户需安装 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)

## 安装与运行

```bash
cd xuetang-helper
pnpm install

# 开发模式（同时启动 Vite 和 Tauri）
pnpm tauri:dev

# 生产打包
pnpm tauri:build
```

首次运行 `pnpm tauri:dev` 时，Cargo 会自动下载所有依赖；冷启动需要数分钟。

## 大模型配置

在「设置」页面中填入 OpenAI 兼容服务的 base url、api key 与模型名。常见的可用配置：

| 提供商 | base_url 示例 | model 示例 |
| --- | --- | --- |
| OpenAI 官方 | `https://api.openai.com/v1` | `gpt-4o-mini` |
| DeepSeek | `https://api.deepseek.com` | `deepseek-chat` |
| 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | `glm-4-flash` |
| 月之暗面 | `https://api.moonshot.cn/v1` | `moonshot-v1-8k` |
| 本地 vLLM/Ollama 反代 | `http://127.0.0.1:8000/v1` | 视部署而定 |

调用形式严格遵循 OpenAI `POST /chat/completions`：

```json
{
  "model": "...",
  "messages": [
    { "role": "system", "content": "..." },
    { "role": "user", "content": "题型 + 题干 + 选项" }
  ],
  "temperature": 0.1
}
```

## 功能与对应接口对照

| 功能 | 接口 |
| --- | --- |
| 扫码登录 | `wss://www.xuetangx.com/wsapp/` + `POST /api/v1/u/login/wx/` |
| 会话校验 | `GET /api/v1/u/login/check_is_l/` |
| 用户信息 | `GET /api/v1/u/user/basic_profile/` |
| 我的课程 | `GET /api/v1/lms/user/user-courses/` |
| 章节树 | `GET /api/v1/lms/kg/kg_learn_chapter/` |
| 节点详情 | `GET /api/v1/lms/learn/leaf_info/{cid}/{leaf}/` |
| 课程进度 | `GET /api/v1/lms/learn/course/schedule` |
| 视频元数据 | `GET /api/v1/lms/service/playurl/{ccid}/?appid=10000` |
| 视频心跳 | `POST /video-log/heartbeat/` |
| 讨论话题（旧） | `GET /api/v1/lms/forum/unit/discussion/?topic_type=0&...` |
| 讨论话题（带分加） | `GET /api/v1/lms/forum/unit/discussion/?topic_type=4&...` |
| 评论列表 | `GET /api/v1/lms/forum/comment/list/{topic_id}/` |
| 发表评论 | `POST /api/v1/lms/forum/comment/` |
| 图文/讨论"已完成"上报 | `POST /api/v1/lms/learn/chapter/schedule` |
| 习题列表 | `GET /api/v1/lms/exercise/get_exercise_list/{eid}/{sku}/` |
| 提交答案 | `POST /api/v1/lms/exercise/problem_apply/` |

## `leaf_type` 速查

学堂在线 `kg_learn_chapter` 返回的章节树中，每个 leaf 的 `leaf_type` 字段对应的语义：

| leaf_type | 类型 | 完成机制 |
| --- | --- | --- |
| 0 | 视频 | 视频心跳；底下的讨论区 `topic_type=0`，无分数 |
| 3 | 图文（article） | `POST chapter/schedule` 即可标记完成 |
| 4 | 讨论（带分加） | 拉 `unit/discussion?topic_type=4` → `POST comment` → `POST chapter/schedule` |
| 6 | 习题 | 走 exercise 体系（`get_exercise_list` / `problem_apply`） |

## 数据存储

账号 cookie、设置等保存在 Tauri 的应用数据目录：

- Windows: `%APPDATA%/com.captain.xuetanghelper/xuetang-helper.store.json`
- macOS: `~/Library/Application Support/com.captain.xuetanghelper/xuetang-helper.store.json`
- Linux: `~/.config/com.captain.xuetanghelper/xuetang-helper.store.json`

本地题库（题面、选项、学堂确认答案、命中次数）单独存到同目录下的 `xuetang-helper.bank.json`，
方便单独备份 / 导入导出 / 一键清空，不会污染账号与设置文件。

## 项目结构

```
xuetang-helper/
├── .github/workflows/        # GitHub Actions（tag 自动发布）
├── src/                      # React + Tailwind 前端（Apple 设计语言）
│   ├── pages/                # Login / Home / Courses / Video / Forum / Homework / Bank / Accounts / Settings / About
│   ├── components/           # Sidebar / 通用 UI（Pill、Card、Tile、Capsule） / icons（BookIcon、RefreshIcon 等 SVG）
│   └── lib/
│       ├── api.ts            # Tauri invoke 封装 + 事件订阅（bank://updated 等）
│       └── videoState.ts     # 全局视频任务状态
└── src-tauri/
    ├── tauri.conf.json
    └── src/
        ├── lib.rs / main.rs
        ├── state.rs          # 全局状态 + 持久化 + task_semaphore + 题库 RwLock
        ├── accounts.rs       # 账号 / Cookie 模型
        ├── client.rs         # reqwest + cookie_store
        ├── login.rs          # WSS 扫码登录
        ├── courses.rs        # 课程 / 章节
        ├── video.rs          # 心跳任务 / 队列 / 按 user_id 终止任务
        ├── forum.rs          # 讨论评论（topic_type 区分）
        ├── article.rs        # 图文 chapter/schedule 完成
        ├── exercise.rs       # 习题 + 自动提交 + 提取学堂下发的标准答案
        ├── bank.rs           # 本地题库：双索引、sha256 题面哈希、JSON 导入导出
        ├── ai.rs             # OpenAI 兼容调用 + 按题型 prompt
        └── commands.rs       # Tauri 命令汇总
```

## 自动构建与发布（GitHub Actions）

仓库提供 `.github/workflows/release.yml`：**推送 `v*` 形式的 tag 即触发**，自动在 GitHub-hosted Runner 上跨三平台构建 Tauri 安装包并发布到 GitHub Release。

矩阵覆盖：

| 平台 | 产物 |
| --- | --- |
| Windows 2022 | `.msi`（推荐）、`.exe`（NSIS） |
| macOS（Apple Silicon） | `.dmg`、`.app.tar.gz`（aarch64） |
| macOS（Intel） | `.dmg`、`.app.tar.gz`（x86_64） |
| Ubuntu 22.04 | `.deb`、`.AppImage`、`.rpm` |

**发布流程**：

```bash
# 1. 准备一次正常的 commit
git commit -m "..."

# 2. 同步 package.json / tauri.conf.json / Cargo.toml 三处 version 字段
#    然后打一个语义化版本 tag：
git tag v1.0.6
git push origin main --follow-tags
```

Push tag 后到 GitHub 仓库的 **Actions** 标签页查看进度；构建完成后会自动创建 Release 草稿/正式发布，每个 runner 的产物都会被 attach。如需手动重跑，可在 Actions 页面用 `workflow_dispatch` 触发（不会创建 Release，仅作调试）。

工作流执行所需的 `GITHUB_TOKEN` 由 Actions 自动注入，无需额外配置 Secret。

> 若希望启用代码签名（macOS notarization / Windows signtool），在 `release.yml` 的 `env` 段加入对应的 secret（参考 [tauri-action 文档](https://github.com/tauri-apps/tauri-action)）。

## 风险与免责

- 学堂在线平台规则可能随时变化，本项目可能因服务端调整而失效。
- 自动评论、自动作业、批量图文/讨论标记可能违反课程要求，使用前请评估后果。
- 大模型回答可能不准，自动提交答案前请自行复核。
- 本仓库不收集、上传任何账号信息，全部数据仅保存在你的本地。

## License

[MIT](LICENSE) © 2026 hor1zon777

> 开源协议解决的是**代码版权**问题，并不为使用本程序产生的合规风险（账号封禁、
> 学籍处分、违反平台 ToS 等）提供任何免责保护。这部分见上方「风险与免责」
> 以及应用内「关于」页的完整免责协议。
