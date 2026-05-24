import { useEffect, useState } from "react";
import { getVersion, getTauriVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { Card, SectionTitle } from "../components/ui";

const GITHUB_URL = "https://github.com/hor1zon777/Xuetangx";
const AUTHOR = "hor1zon777 (Captain)";

/**
 * "关于"页面：展示应用基本信息（版本号、作者、仓库地址）以及免责协议。
 *
 * 版本号通过 Tauri 的 [`getVersion`] 在运行时读取 `tauri.conf.json` 里的值，
 * 这样升级时只要改一处（配置文件），UI 上展示的版本号会跟着自动同步，
 * 不会出现"硬编码字符串忘了改"的脱节问题。
 */
export function AboutPage() {
  const [appVersion, setAppVersion] = useState<string>("…");
  const [tauriVer, setTauriVer] = useState<string>("…");

  useEffect(() => {
    // 两个 Promise 并发，结果到位再 setState。失败时只渲染问号而不阻断页面。
    getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("?"));
    getTauriVersion()
      .then(setTauriVer)
      .catch(() => setTauriVer("?"));
  }, []);

  /**
   * 在系统默认浏览器中打开外链。
   *
   * Tauri 2 的 webview 拦截了普通 `window.open` / `<a target="_blank">`：
   * 直接用它们要么没反应，要么试图在当前 webview 内导航——都不是我们想要的。
   * 正确做法是调用 tauri-plugin-shell 的 `open` 命令；项目已在
   * `capabilities/default.json` 里授予 `shell:allow-open`，Rust 侧也注册了
   * `tauri_plugin_shell::init()`，所以直接 invoke 即可，不必引入额外的
   * `@tauri-apps/plugin-shell` JS 包。
   */
  const openExternal = async (url: string) => {
    try {
      await invoke("plugin:shell|open", { path: url });
    } catch (e) {
      // 兜底：极端情况下（权限缺失等）退回到 window.open，避免完全无反应。
      console.warn("shell:open 失败，回退到 window.open:", e);
      window.open(url, "_blank", "noopener,noreferrer");
    }
  };

  return (
    <div>
      <SectionTitle title="关于" subtitle="项目信息、版本号与免责声明。" />

      <div className="px-12 grid grid-cols-1 lg:grid-cols-2 gap-6 pb-8">
        <Card>
          <div className="font-display text-tagline mb-3">应用信息</div>
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-body">
            <dt className="text-ink-muted-80">应用名称</dt>
            <dd className="text-ink">学堂在线助手</dd>

            <dt className="text-ink-muted-80">版本号</dt>
            <dd className="text-ink font-mono">v{appVersion}</dd>

            <dt className="text-ink-muted-80">Tauri 运行时</dt>
            <dd className="text-ink font-mono">v{tauriVer}</dd>

            <dt className="text-ink-muted-80">作者</dt>
            <dd className="text-ink">{AUTHOR}</dd>

            <dt className="text-ink-muted-80">许可协议</dt>
            <dd>
              <button
                type="button"
                className="text-link"
                onClick={() => openExternal(`${GITHUB_URL}/blob/main/LICENSE`)}
              >
                GPL-3.0 License
              </button>
            </dd>

            <dt className="text-ink-muted-80">开源仓库</dt>
            <dd>
              <button
                type="button"
                className="text-link break-all"
                onClick={() => openExternal(GITHUB_URL)}
              >
                {GITHUB_URL}
              </button>
            </dd>
          </dl>
          <div className="mt-4 text-caption text-ink-muted-80 leading-relaxed">
            欢迎到仓库提交 Issue / Pull Request；如果对你有帮助，欢迎 Star。
          </div>
        </Card>

        <Card>
          <div className="font-display text-tagline mb-3">免责声明</div>
          <ol className="list-decimal pl-5 space-y-2 text-caption text-ink-muted-80 leading-relaxed">
            <li>
              本程序仅用于<strong className="text-ink">个人学习交流与技术研究</strong>，
              不得用于任何商业用途，亦不得用于代刷课、代考、批量违规获取学分等
              违反学校 / 课程方规定的行为。
            </li>
            <li>
              本程序模拟学堂在线的官方 HTTP / WSS 接口，所有操作（视频心跳、
              讨论评论、图文标记完成、自动答题、章节进度上报）皆为<strong className="text-ink">
              使用者自行触发</strong>。任何因使用本程序导致的账号封禁、成绩取消、
              学籍处分、法律责任等后果，由使用者本人承担。
            </li>
            <li>
              本程序<strong className="text-ink">不收集、不上传、不存储</strong>任何用户的账号、
              Cookie、答题记录或学习数据；全部信息仅保存在你的本地数据目录
              （Windows: <code>%APPDATA%/com.captain.xuetanghelper/</code>）。
              卸载或清除该目录即可销毁全部数据。
            </li>
            <li>
              自动答题功能依赖第三方 OpenAI 兼容大模型，
              <strong className="text-ink">回答可能不准确</strong>，
              请在自动提交前自行核对。提交答案触发的"分数"由学堂在线服务端
              依据课程规则计算，本程序不对结果作任何承诺。
            </li>
            <li>
              学堂在线平台的接口、风控策略、计分逻辑可能随时调整，
              本程序<strong className="text-ink">不保证持续可用</strong>，
              也不承担因服务端变更而导致的失效、数据异常等问题。
            </li>
            <li>
              本项目代码以现状（AS-IS）方式提供，
              <strong className="text-ink">不附带任何明示或暗示的担保</strong>。
              开发者保留对项目的解释权与终止维护的权利。
            </li>
            <li>
              下载、安装或运行本程序，即视为你<strong className="text-ink">已完整阅读
              并接受</strong>以上全部条款。如不同意，请立刻停止使用并删除本程序。
            </li>
          </ol>
        </Card>
      </div>
    </div>
  );
}
