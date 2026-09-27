// Preserve the settings-first desktop workflow: client switch, mode tabs, profile fields,
// tab-local save, and a separate bottom launch action. Temporary edits never leave this
// window's memory, and both clients are edited from one draft.
import { useEffect, useRef, useState } from "react";
import {
  AlertDialog,
  Button,
  Select,
  Tabs,
  TextField,
  Theme,
} from "@radix-ui/themes";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Agent,
  AgentDraft,
  Draft,
  Fields,
  Mode,
  Progress,
  View,
} from "./types";

type Confirmation = {
  title: string;
  body: string;
  action: string;
  run: () => void;
};
const clone = <T,>(v: T): T => structuredClone(v);
const same = (a: unknown, b: unknown): boolean =>
  JSON.stringify(a) === JSON.stringify(b);
const LABEL: Record<Agent, string> = { codex: "Codex", claude: "Claude" };
function blank(agent: Agent): AgentDraft {
  return {
    agent,
    mode: "official",
    library: {
      official_accounts: [],
      custom_providers: [],
      selected_official: "",
      selected_custom: "",
      official_proxy_url: "",
      official_model: "",
      official_effort: "",
    },
    custom_fields: {},
  };
}

export default function App() {
  const [saved, setSaved] = useState<View>();
  const [draft, setDraft] = useState<Draft>();
  const [agent, setAgent] = useState<Agent>("codex");
  const [busy, setBusy] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [progress, setProgress] = useState<Progress>();
  const [confirmation, setConfirmation] = useState<Confirmation>();
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
  );
  // 两个客户端的草稿一起参与比较：切换客户端不算修改，也不丢另一个客户端的编辑。
  const dirty = !!draft && !!saved && !same(draft, saved.draft);
  const closeRef = useRef<() => void>(() => {});
  const perform = async (label: string, operation: () => Promise<void>) => {
    setBusy(label);
    setError("");
    setMessage("");
    try {
      await operation();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
      setProgress(undefined);
    }
  };
  const install = (view: View) => {
    setSaved(view);
    setDraft(clone(view.draft));
  };
  const reload = () =>
    perform("读取配置", async () => {
      install(await invoke<View>("load"));
    });
  const close = () => {
    if (busy) {
      setError("操作正在执行，请等待完成后关闭。");
      return;
    }
    if (dirty)
      setConfirmation({
        title: "放弃未保存的修改？",
        body: "已保存的配置会保留。关闭不会停止正在运行的客户端。",
        action: "放弃修改并关闭",
        run: () => {
          void invoke("close");
        },
      });
    else void invoke("close");
  };
  closeRef.current = close;
  useEffect(() => {
    let disposed = false;
    const subscriptions = [
      listen("request-close", () => closeRef.current()),
      listen<Progress>("repair-progress", (e) => setProgress(e.payload)),
    ];
    void Promise.all(subscriptions)
      .then(async (unlisten) => {
        if (disposed) {
          unlisten.forEach((fn) => fn());
          return;
        }
        await reload();
      })
      .catch((e) => setError(String(e)));
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === "Escape" &&
        !document.querySelector('[role="alertdialog"][data-state="open"]') &&
        !document.querySelector('[role="listbox"]')
      ) {
        e.preventDefault();
        e.stopImmediatePropagation();
        closeRef.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      disposed = true;
      void Promise.all(subscriptions).then((list) =>
        list.forEach((fn) => fn()),
      );
      window.removeEventListener("keydown", onKey, true);
    };
  }, []);

  const edit = (change: (active: AgentDraft, draft: Draft) => void) => {
    if (!draft || busy) return;
    const next = clone(draft);
    change(next[agent], next);
    setDraft(next);
    setMessage("");
    setError("");
  };
  const active = draft?.[agent];
  const mode = active?.mode ?? "official";
  const collection =
    mode === "official" ? "official_accounts" : "custom_providers";
  const selection =
    mode === "official" ? "selected_official" : "selected_custom";
  const profiles = active?.library[collection] ?? [];
  const id = active?.library[selection] ?? "";
  const profile = profiles.find((p) => p.id === id);
  const fields = active?.custom_fields[id];
  const add = (copy = false) =>
    edit((current) => {
      const newId = crypto.randomUUID().slice(0, 8);
      const name = copy
        ? `${profile?.name || "未命名配置"} 副本`
        : mode === "official"
          ? "官方账号"
          : "自定义 API";
      current.library[collection].push({
        ...(copy ? clone(profile!) : {}),
        id: newId,
        name,
      });
      current.library[selection] = newId;
      if (mode === "custom")
        current.custom_fields[newId] = copy
          ? clone(fields!)
          : {
              provider_name: "custom",
              base_url: "",
              api_key: "",
              model: "",
              effort: "medium",
            };
    });
  const remove = () => {
    if (profiles.length === 1) {
      setError(
        "当前模式只有一项配置，删除后无法应用。请先添加其他配置，再删除这一项。",
      );
      return;
    }
    setConfirmation({
      title: "删除这项配置？",
      body: "删除只改变当前草稿，点击保存配置后才生效。",
      action: "删除",
      run: () =>
        edit((current) => {
          current.library[collection] = current.library[collection].filter(
            (p) => p.id !== id,
          );
          current.library[selection] = current.library[collection][0]?.id ?? "";
          if (mode === "custom") delete current.custom_fields[id];
        }),
    });
  };
  const field = (key: keyof Fields, value: string) =>
    edit((current) => {
      current.custom_fields[id][key] = value;
    });
  const save = () =>
    perform("保存配置", async () => {
      install(
        await invoke<View>("save", {
          revision: saved!.revision,
          draft,
          agent,
        }),
      );
      setMessage("已保存配置；尚未启动客户端。");
    });
  const launch = () => {
    if (dirty) {
      setError("有未保存修改，请先点击当前页的“保存配置”。");
      return;
    }
    void perform("启动客户端", async () => {
      await invoke("launch", { revision: saved!.revision, draft, agent });
    });
  };
  const repair = () =>
    setConfirmation({
      title: "修复本地历史对话？",
      body: "将 sessions、archived_sessions 和 SQLite 中历史对话的提供者 ID 改为 custom。修复前会创建备份，不修改提供者显示名称，也不保存当前表单。请先关闭正在写入历史的 Codex 客户端。",
      action: "备份并修复",
      run: () => {
        void perform("修复对话", async () => {
          const result = await invoke<{
            changed: number;
            backup: string | null;
          }>("repair");
          setMessage(
            result.changed
              ? `已修复 ${result.changed} 项。备份：${result.backup}`
              : "历史对话已关联 custom，无需修改。",
          );
        });
      },
    });

  return (
    <Theme
      appearance={theme}
      accentColor="iris"
      grayColor="slate"
      radius="medium"
      scaling="100%"
    >
      <main className="app">
        <header>
          <div>
            <h1>连接设置</h1>
            <p>管理 Codex 与 Claude 的官方账号和自定义 API，保存后再启动。</p>
          </div>
          <Button
            className="theme-toggle"
            variant="soft"
            color="gray"
            aria-label={theme === "dark" ? "浅色" : "深色"}
            title={theme === "dark" ? "切换到浅色外观" : "切换到深色外观"}
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          >
            <span className="theme-track" aria-hidden="true">
              <span className="theme-thumb" />
              <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="4" /><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5" /></svg>
              <svg viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 0 1 8.5 4 8.5 8.5 0 1 0 20 15.5Z" /></svg>
            </span>
            <span>{theme === "dark" ? "浅色" : "深色"}</span>
          </Button>
        </header>
        <section className="content" aria-busy={!!busy}>
          {active ? (
            <>
            <div className="agent-switch" role="tablist" aria-label="客户端">
              {(["codex", "claude"] as const).map((name) => (
                <Button
                  key={name}
                  variant={agent === name ? "solid" : "soft"}
                  disabled={!!busy}
                  role="tab"
                  aria-selected={agent === name}
                  onClick={() => setAgent(name)}
                >
                  <span className={`agent-icon ${name}`} aria-hidden="true">
                    {name === "codex" ? "✦" : "☁"}
                  </span>
                  <span>{LABEL[name]}</span>
                </Button>
              ))}
            </div>
            <Tabs.Root
              value={mode}
              onValueChange={(value) =>
                edit((current) => {
                  current.mode = value as Mode;
                })
              }
            >
              <Tabs.List aria-label="连接方式" className="mode-switch" data-mode={mode}>
                <Tabs.Trigger value="official" disabled={!!busy}>
                  官方账号
                </Tabs.Trigger>
                <Tabs.Trigger value="custom" disabled={!!busy}>
                  自定义 API
                </Tabs.Trigger>
              </Tabs.List>
              <Tabs.Content value={mode} className="settings">
                <div className="profile-row">
                  <label className="profile-select">
                    <span>{mode === "official" ? "官方账号" : "API 配置"}</span>
                    <Select.Root
                      value={id || undefined}
                      disabled={!profiles.length || !!busy}
                      onValueChange={(value) =>
                        edit((current) => {
                          current.library[selection] = value;
                        })
                      }
                    >
                      <Select.Trigger
                        placeholder="尚未添加配置"
                        aria-label="选择配置"
                      />
                      <Select.Content>
                        {profiles.map((p) => (
                          <Select.Item key={p.id} value={p.id}>
                            {p.name?.trim() || "未命名配置"}
                          </Select.Item>
                        ))}
                      </Select.Content>
                    </Select.Root>
                  </label>
                  <div className="profile-actions">
                    <Button
                      variant="soft"
                      onClick={() => add()}
                      disabled={!!busy}
                    >
                      添加
                    </Button>
                    {mode === "custom" && (
                      <Button
                        variant="soft"
                        onClick={() => add(true)}
                        disabled={!profile || !!busy}
                      >
                        复制
                      </Button>
                    )}
                    <Button
                      variant="soft"
                      color="gray"
                      disabled={!profile || !!busy}
                      onClick={remove}
                    >
                      删除
                    </Button>
                  </div>
                </div>
                {profile ? (
                  <div className="form">
                    <Field
                      label="配置名称"
                      value={profile.name ?? ""}
                      disabled={!!busy}
                      onChange={(v) =>
                        edit((current) => {
                          current.library[collection].find(
                            (p) => p.id === id,
                          )!.name = v;
                        })
                      }
                    />
                    {mode === "official" ? (
                      <>
                        <div className="field account-status">
                          <span>账号状态</span>
                          <strong>
                            {profile.email ||
                              profile.account_name ||
                              "登录由官方客户端完成"}
                          </strong>
                          <p>
                            {profile.official_auth
                              ? "已保存本地凭据；账号是否有效由客户端验证。"
                              : "保存并启动后，在官方客户端登录此账号。"}
                          </p>
                        </div>
                        <Field
                          label="官方模型（可留空）"
                          value={active.library.official_model ?? ""}
                          disabled={!!busy}
                          onChange={(v) =>
                            edit((current) => {
                              current.library.official_model = v;
                            })
                          }
                        />
                        <EffortSelect
                          label="思考层级"
                          value={active.library.official_effort ?? "medium"}
                          disabled={!!busy}
                          onChange={(v) => edit((current) => { current.library.official_effort = v; })}
                        />
                        <Field
                          label="HTTP 代理"
                          value={active.library.official_proxy_url ?? ""}
                          placeholder="http://127.0.0.1:7890"
                          hint="所有官方账号共用。留空不设置独立代理；不修改系统代理。"
                          disabled={!!busy}
                          onChange={(v) =>
                            edit((current) => {
                              current.library.official_proxy_url = v;
                            })
                          }
                        />
                      </>
                    ) : (
                      fields && (
                        <>
                          <Field
                            label="提供者显示名称"
                            value={fields.provider_name}
                            disabled={!!busy}
                            onChange={(v) => field("provider_name", v)}
                          />
                          <Field
                            wide
                            label="API 地址"
                            value={fields.base_url}
                            placeholder={agent === "claude" ? "https://api.example.com" : "https://api.example.com/v1"}
                            disabled={!!busy}
                            onChange={(v) => field("base_url", v)}
                          />
                          <Field
                            wide
                            label="API Key"
                            value={fields.api_key}
                            disabled={!!busy}
                            onChange={(v) => field("api_key", v)}
                          />
                          <Field
                            label="模型"
                            value={fields.model}
                            disabled={!!busy}
                            onChange={(v) => field("model", v)}
                          />
                          <EffortSelect
                            label="思考层级"
                            value={fields.effort || "medium"}
                            disabled={!!busy}
                            onChange={(v) => field("effort", v)}
                          />
                        </>
                      )
                    )}
                  </div>
                ) : (
                  <div className="empty">
                    <h2>
                      尚未添加{mode === "official" ? "官方账号" : "API 配置"}
                    </h2>
                    <p>点击“添加”，填写配置后保存。</p>
                  </div>
                )}
                <div className="save-row">
                  <Button
                    className="primary-action"
                    onClick={() => void save()}
                    disabled={!profile || !!busy}
                  >
                    {busy === "保存配置" ? "正在保存…" : "保存配置"}
                  </Button>
                  <span>{dirty ? "有未保存修改" : "配置未修改"}</span>
                  {mode === "custom" && agent === "codex" && (
                    <Button
                      variant="outline"
                      color="gray"
                      onClick={repair}
                      disabled={!!busy}
                    >
                      修复对话
                    </Button>
                  )}
                </div>
              </Tabs.Content>
            </Tabs.Root>
            </>
          ) : (
            <p>{busy || "配置尚未载入。可点击“重新读取”重试。"}</p>
          )}
          {progress && (
            <div className="progress" role="status">
              <span>
                {progress.phase} {progress.completed}/{progress.total}
              </span>
              <progress
                value={progress.completed}
                max={Math.max(1, progress.total)}
              />
            </div>
          )}
          {error && (
            <div className="error" role="alert">
              {error}
            </div>
          )}
          {message && (
            <div className="message" role="status">
              {message}
            </div>
          )}
        </section>
        <footer>
          <div className="utility">
            <Button
              size="1"
              variant="ghost"
              color="gray"
              disabled={!!busy}
              onClick={() =>
                void perform("打开下载页面", async () => {
                  await invoke("open_download", { updates: false, agent });
                })
              }
            >
              安装客户端
            </Button>
            <Button
              size="1"
              variant="ghost"
              color="gray"
              disabled={!!busy}
              onClick={() =>
                void perform("检查更新", async () => {
                  await invoke("open_download", { updates: true, agent });
                })
              }
            >
              检查更新
            </Button>
            <Button
              size="1"
              variant="ghost"
              color="gray"
              disabled={!!busy}
              onClick={() =>
                dirty
                  ? setConfirmation({
                      title: "放弃草稿并重新读取？",
                      body: "重新读取会丢弃未保存的修改，读取其他程序写入的最新配置。",
                      action: "重新读取",
                      run: () => {
                        void reload();
                      },
                    })
                  : void reload()
              }
            >
              重新读取
            </Button>
          </div>
          <div className="bottom-actions">
            <Button
              variant="soft"
              color="gray"
              onClick={close}
              disabled={!!busy}
            >
              关闭
            </Button>
            <Button className="launch-action" onClick={launch} disabled={!saved || !!busy}>
              {busy === "启动客户端" ? "正在启动…" : "启动"}
            </Button>
          </div>
        </footer>
        <AlertDialog.Root
          open={!!confirmation}
          onOpenChange={(open) => {
            if (!open) setConfirmation(undefined);
          }}
        >
          <AlertDialog.Content maxWidth="460px">
            <AlertDialog.Title>{confirmation?.title}</AlertDialog.Title>
            <AlertDialog.Description>
              {confirmation?.body}
            </AlertDialog.Description>
            <div className="dialog-actions">
              <AlertDialog.Cancel>
                <Button variant="soft" color="gray" autoFocus>
                  继续编辑
                </Button>
              </AlertDialog.Cancel>
              <AlertDialog.Action>
                <Button
                  className="primary-action"
                  onClick={() => {
                    const run = confirmation?.run;
                    setConfirmation(undefined);
                    run?.();
                  }}
                >
                  {confirmation?.action}
                </Button>
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Root>
      </main>
    </Theme>
  );
}
function Field({
  label,
  value,
  onChange,
  disabled,
  placeholder,
  hint,
  wide,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled: boolean;
  placeholder?: string;
  hint?: string;
  wide?: boolean;
}) {
  return (
    <label className={`field${wide ? " wide" : ""}${hint ? " with-hint" : ""}`}>
      <span>{label}</span>
      <TextField.Root
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        placeholder={placeholder}
        autoComplete="off"
        spellCheck={false}
      />
      {hint && <span className="hint">{hint}</span>}
    </label>
  );
}

function EffortSelect({
  label,
  value,
  onChange,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled: boolean;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      <Select.Root value={value} onValueChange={onChange} disabled={disabled}>
        <Select.Trigger aria-label={label} />
        <Select.Content>
          <Select.Item value="low">低 · 快速</Select.Item>
          <Select.Item value="medium">中 · 平衡</Select.Item>
          <Select.Item value="high">高 · 深入</Select.Item>
          <Select.Item value="xhigh">极高 · 最深入</Select.Item>
        </Select.Content>
      </Select.Root>
    </label>
  );
}
