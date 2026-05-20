# 学堂在线助手（xuetang-helper）

> Rust + Tauri v2 + React 实现的桌面端学堂在线自动学习工具。界面遵循 Apple 设计语言（详见 `apple/DESIGN.md`）。

## 功能

- 微信扫码登录（基于学堂在线官方 `wss://www.xuetangx.com/wsapp/`）
- 账号本地缓存与一键切换
- 课程 / 章节 / 节点遍历（支持视频、习题、讨论）
- 自动观看视频（按官方心跳协议 `/video-log/heartbeat/`）
  - 支持倍速、批量任务、停止、进度展示
- 讨论区批量发评论（自动获取 `topic_id` 与 `to_user`）
- 自动作业：拉取题目 → 询问 OpenAI 兼容大模型 → 提交答案 → 展示得分
- 设置面板：配置 OpenAI 兼容接口、心跳间隔、默认倍速、默认评论文案

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
| 视频心跳 | `POST /video-log/heartbeat/` |
| 讨论话题 | `GET /api/v1/lms/forum/unit/discussion/` |
| 发表评论 | `POST /api/v1/lms/forum/comment/` |
| 习题列表 | `GET /api/v1/lms/exercise/get_exercise_list/{eid}/{sku}/` |
| 提交答案 | `POST /api/v1/lms/exercise/problem_apply/` |

## 数据存储

账号 cookie、设置等保存在 Tauri 的应用数据目录：

- Windows: `%APPDATA%/com.captain.xuetanghelper/xuetang-helper.store.json`
- macOS: `~/Library/Application Support/com.captain.xuetanghelper/xuetang-helper.store.json`
- Linux: `~/.config/com.captain.xuetanghelper/xuetang-helper.store.json`

## 项目结构

```
xuetang-helper/
├── src/                # React + Tailwind 前端（Apple 设计语言）
│   ├── pages/          # Login / Home / Courses / Video / Forum / Homework / Accounts / Settings
│   ├── components/     # Sidebar / 通用 UI（Pill、Card、Tile、Capsule）
│   └── lib/api.ts      # Tauri invoke 封装
└── src-tauri/
    ├── tauri.conf.json
    └── src/
        ├── lib.rs / main.rs
        ├── state.rs      # 全局状态 + 持久化
        ├── accounts.rs   # 账号 / Cookie 模型
        ├── client.rs     # reqwest + cookie_store
        ├── login.rs      # WSS 扫码登录
        ├── courses.rs    # 课程 / 章节
        ├── video.rs      # 心跳任务
        ├── forum.rs      # 讨论评论
        ├── exercise.rs   # 习题 + 自动提交
        ├── ai.rs         # OpenAI 兼容调用
        └── commands.rs   # Tauri 命令汇总
```

## 风险与免责

- 学堂在线平台规则可能随时变化，本项目可能因服务端调整而失效。
- 自动评论、自动作业可能违反课程要求，使用前请评估后果。
- 大模型回答可能不准，自动提交答案前请自行复核。
- 本仓库不收集、上传任何账号信息，全部数据仅保存在你的本地。
