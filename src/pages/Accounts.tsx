import { useEffect, useState } from "react";
import { api, type Account } from "../lib/api";
import { Card, Pill, SectionTitle } from "../components/ui";

export function AccountsPage({
  onChanged,
  onLoginAgain,
}: {
  onChanged: () => void;
  onLoginAgain: () => void;
}) {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [current, setCurrent] = useState<Account | null>(null);

  const refresh = async () => {
    const [list, cur] = await Promise.all([api.listAccounts(), api.currentAccount()]);
    setAccounts(list);
    setCurrent(cur);
  };

  useEffect(() => {
    refresh();
  }, []);

  const switchTo = async (uid: number) => {
    await api.switchAccount(uid);
    await refresh();
    onChanged();
  };

  const remove = async (uid: number) => {
    if (!confirm("确定移除该账号的本地缓存？")) return;
    await api.removeAccount(uid);
    await refresh();
    onChanged();
  };

  return (
    <div>
      <SectionTitle title="账号管理" subtitle="本地缓存的账号会自动保留登录态，支持快速切换。" />
      <div className="px-12 grid grid-cols-1 md:grid-cols-2 gap-6">
        {accounts.map((a) => (
          <Card key={a.user_id}>
            <div className="flex items-center gap-4">
              {a.avatar ? (
                <img
                  src={a.avatar}
                  className="w-12 h-12 rounded-full object-cover"
                  alt=""
                />
              ) : (
                <div className="w-12 h-12 rounded-full bg-parchment" />
              )}
              <div className="flex-1 min-w-0">
                <div className="font-display text-tagline truncate">{a.nickname}</div>
                <div className="text-caption text-ink-muted-48">
                  user_id：{a.user_id} · 登录时间：
                  {new Date(a.login_time * 1000).toLocaleString()}
                </div>
              </div>
              {current?.user_id === a.user_id && (
                <span className="text-caption text-action-blue">当前</span>
              )}
            </div>
            <div className="mt-4 flex gap-3">
              <Pill onClick={() => switchTo(a.user_id)}>切换</Pill>
              <Pill variant="ghost" onClick={() => remove(a.user_id)}>
                移除
              </Pill>
            </div>
          </Card>
        ))}
        <Card className="flex items-center justify-center text-center">
          <div>
            <div className="font-display text-tagline text-ink">添加新账号</div>
            <p className="text-caption text-ink-muted-80 mt-2 mb-4">
              使用微信扫码登录另一个学堂账号。
            </p>
            <Pill onClick={onLoginAgain}>去扫码登录</Pill>
          </div>
        </Card>
      </div>
    </div>
  );
}
