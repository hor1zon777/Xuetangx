import { useEffect, useRef, useState } from "react";
import { api, onLoginEvents } from "../lib/api";
import { Card, Pill, Spinner } from "../components/ui";
import { ShieldIcon, SparkIcon, WeChatIcon } from "../components/icons";

export function LoginPage({
  onLoggedIn,
  onCancel,
}: {
  onLoggedIn: () => void;
  /**
   * 可选：在已存在登录账号的场景下（例如从账号管理页跳转过来添加新账号），
   * 点击“取消”按钮应当返回上一级页面，而不是停留在扫码页。
   * 不传则表示当前是首次登录入口，取消仅终止后端会话，UI 保持登录页。
   */
  onCancel?: () => void;
}) {
  const [qrTicket, setQrTicket] = useState<string | null>(null);
  const [expireSeconds, setExpireSeconds] = useState<number>(60);
  const [scanned, setScanned] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const unlisten = useRef<(() => void) | undefined>();
  const timer = useRef<number | null>(null);
  const started = useRef(false);

  // 取消登录：终止后端 WebSocket 会话；如果父级提供了 onCancel，则同时返回上一级。
  // 不依赖后端 login://cancelled 事件来切换 UI，避免事件丢失导致按钮“无反应”。
  // 先停掉本地定时器和事件订阅并触发父级返回，再异步发起后端取消，
  // 这样即使后端响应慢，用户也能立即感知到“取消”生效。
  const handleCancel = () => {
    if (timer.current) {
      window.clearInterval(timer.current);
      timer.current = null;
    }
    unlisten.current?.();
    unlisten.current = undefined;
    setLoading(false);
    setScanned(false);
    setQrTicket(null);
    // 后端会话异步关闭，失败也不影响 UI 返回（下一次登录会重建会话）。
    api.cancelLogin().catch((e) => console.warn("取消登录失败：", e));
    onCancel?.();
  };

  useEffect(() => {
    if (!started.current) {
      started.current = true;
      start();
    }
    return () => {
      unlisten.current?.();
      if (timer.current) window.clearInterval(timer.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const startCountdown = (sec: number) => {
    if (timer.current) window.clearInterval(timer.current);
    setExpireSeconds(sec);
    timer.current = window.setInterval(() => {
      setExpireSeconds((s) => {
        if (s <= 1) {
          window.clearInterval(timer.current!);
          // 自动刷新二维码
          start();
          return 0;
        }
        return s - 1;
      });
    }, 1000);
  };

  const start = async () => {
    setError(null);
    setScanned(false);
    setQrTicket(null);
    setLoading(true);
    unlisten.current?.();
    unlisten.current = await onLoginEvents({
      onQr: (p) => {
        setQrTicket(p.ticket);
        startCountdown(p.expire_seconds);
        setLoading(false);
      },
      onScanned: () => setScanned(true),
      onSuccess: () => {
        setLoading(false);
        if (timer.current) window.clearInterval(timer.current);
        onLoggedIn();
      },
      onError: (p) => {
        setError(p.message);
        setLoading(false);
      },
      onCancelled: () => {
        setLoading(false);
      },
    });
    try {
      await api.startLogin();
    } catch (e: any) {
      setError(String(e));
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-parchment flex flex-col">
      {/* 顶部品牌区 */}
      <header className="pt-[clamp(40px,8vh,96px)] pb-[clamp(24px,4vh,48px)] px-6 text-center">
        <div className="inline-flex items-center justify-center w-12 h-12 rounded-full bg-action-blue/10 text-action-blue mb-4">
          <SparkIcon className="w-6 h-6" />
        </div>
        <div className="text-caption text-ink-muted-48 tracking-widest uppercase">
          xuetang helper
        </div>
        <h1 className="font-display mt-3 text-ink text-[clamp(34px,6vw,56px)] leading-[1.07] tracking-[-0.02em]">
          登录学堂在线
        </h1>
        <p className="font-text mt-3 text-ink-muted-80 text-[clamp(15px,1.6vw,21px)] leading-[1.3]">
          使用微信扫一扫，开启高效学习的另一种方式
        </p>
      </header>

      {/* 主体卡片区 */}
      <main className="flex-1 flex items-start justify-center px-6 pb-[clamp(40px,8vh,96px)]">
        <Card className="w-full max-w-[420px] flex flex-col items-center text-center">
          {/* 二维码标题（微信图标 + 文案） */}
          <div className="inline-flex items-center gap-2 text-caption text-ink-muted-80 mb-4">
            <WeChatIcon className="w-4 h-4 text-[#07c160]" />
            <span>微信扫码登录</span>
          </div>

          {/* 二维码框 */}
          <div className="relative w-[min(72vw,300px)] aspect-square rounded-lg bg-parchment overflow-hidden flex items-center justify-center">
            {qrTicket ? (
              <img
                src={qrTicket}
                alt="微信二维码"
                className="w-full h-full object-contain"
              />
            ) : (
              <div className="flex flex-col items-center gap-3 text-ink-muted-48 text-caption">
                {loading ? <Spinner /> : null}
                <span>{loading ? "正在获取二维码…" : "等待中"}</span>
              </div>
            )}

            {/* 扫码后的遮罩 */}
            {scanned && qrTicket && (
              <div className="absolute inset-0 bg-white/85 backdrop-blur-sm flex flex-col items-center justify-center gap-2">
                <div className="font-display text-tagline text-ink">已扫描</div>
                <div className="text-caption text-ink-muted-80">
                  请在手机上确认登录
                </div>
              </div>
            )}

            {/* 过期遮罩 */}
            {qrTicket && expireSeconds === 0 && !scanned && (
              <div className="absolute inset-0 bg-white/85 backdrop-blur-sm flex flex-col items-center justify-center gap-3">
                <div className="text-caption text-ink-muted-80">二维码已失效</div>
                <Pill onClick={start}>重新获取</Pill>
              </div>
            )}
          </div>

          {/* 状态文本 */}
          <div className="mt-6 min-h-[24px] text-caption text-ink-muted-80">
            {error ? (
              <span className="text-[#cc2b2b] break-all">{error}</span>
            ) : qrTicket && !scanned ? (
              <>请使用微信扫描二维码，剩余 {expireSeconds} 秒</>
            ) : scanned ? (
              <span className="text-action-blue">登录确认中…</span>
            ) : (
              <span>&nbsp;</span>
            )}
          </div>

          {/* 操作按钮 */}
          <div className="mt-4 flex flex-col items-stretch gap-3 w-full">
            <Pill onClick={start} disabled={loading}>
              {loading ? <Spinner /> : qrTicket ? "刷新二维码" : "获取二维码"}
            </Pill>
            {(qrTicket || onCancel) && (
              <Pill variant="ghost" onClick={handleCancel}>
                取消
              </Pill>
            )}
          </div>
        </Card>
      </main>

      {/* 页脚说明 */}
      <footer className="px-6 pb-8 text-center">
        <div className="inline-flex items-center gap-1.5 text-fine text-ink-muted-48">
          <ShieldIcon className="w-3.5 h-3.5" />
          <span>登录态仅保存在本地，不会上传到任何第三方</span>
        </div>
      </footer>
    </div>
  );
}
