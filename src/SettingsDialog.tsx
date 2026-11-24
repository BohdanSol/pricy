import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type ClaudeStatus, type Settings } from "./api";
import type { useSources } from "./useSources";
import { parseTeam } from "./Workflow";

const MODELS = [
  { value: "", label: "Default (Claude Code setting)" },
  { value: "sonnet", label: "Sonnet — balanced" },
  { value: "opus", label: "Opus — most capable" },
  { value: "haiku", label: "Haiku — fastest" },
];

const CONNECTORS_URL = "https://claude.ai/settings/connectors";

export function SettingsDialog({
  sources: src,
  onClose,
  onError,
}: {
  sources: ReturnType<typeof useSources>;
  onClose: () => void;
  onError: (e: unknown) => void;
}) {
  const [settings, setSettings] = useState<Settings>({ claudePath: "", model: "", team: "" });
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    api.getSettings().then(setSettings).catch(onError);
    api.checkClaude().then(setStatus).catch(onError);
  }, [onError]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function save() {
    try {
      await api.saveSettings(settings);
      onClose();
    } catch (e) {
      onError(e);
    }
  }

  async function test() {
    setChecking(true);
    try {
      await api.saveSettings(settings);
      setStatus(await api.checkClaude());
    } catch (e) {
      onError(e);
    } finally {
      setChecking(false);
    }
  }

  async function attachWhatsapp() {
    const picked = await open({
      multiple: true,
      directory: false,
      title: "Attach WhatsApp chat export",
      filters: [{ name: "WhatsApp chat export", extensions: ["txt"] }],
    });
    if (!picked) return;
    try {
      await api.addWhatsappFiles(Array.isArray(picked) ? picked : [picked]);
      await src.refreshSources();
    } catch (e) {
      onError(e);
    }
  }

  async function removeWhatsapp(name: string) {
    if (!confirm(`Remove "${name}" from Pricy? The original file is not touched.`)) return;
    try {
      await api.removeWhatsappFile(name);
      await src.refreshSources();
    } catch (e) {
      onError(e);
    }
  }

  async function changeConnector(sourceId: Parameters<typeof api.setConnector>[0], name: string) {
    try {
      await api.setConnector(sourceId, name);
      await src.refreshSources();
    } catch (e) {
      onError(e);
    }
  }

  const connectorNames = (src.connectors ?? []).map((c) => c.name);

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Settings</h2>

        <section>
          <h3>Team</h3>
          <p className="muted small">
            Colleagues you send requests to for approval, supplier messages and final answers. One per line, as{" "}
            <code>Name &lt;email&gt;</code> or just the email.
          </p>
          <textarea
            className="team-input"
            rows={4}
            value={settings.team}
            placeholder={"Anna Rossi <anna@company.com>\nmarco@company.com"}
            onChange={(e) => setSettings({ ...settings, team: e.target.value })}
          />
          <p className="muted small">
            {(() => {
              const people = parseTeam(settings.team);
              const lines = settings.team.split(/[\n,;]+/).filter((l) => l.trim()).length;
              const bad = lines - people.length;
              return `${people.length} member${people.length === 1 ? "" : "s"}${bad > 0 ? ` · ${bad} line${bad === 1 ? "" : "s"} without a valid email` : ""}`;
            })()}
          </p>
        </section>

        <section>
          <h3>Data sources</h3>
          <p className="muted small">
            TINT, Slack and Drive use your own claude.ai connectors, read-only: Pricy can search them but never post,
            book or create anything.{" "}
            <a
              href={CONNECTORS_URL}
              onClick={(e) => {
                e.preventDefault();
                openUrl(CONNECTORS_URL);
              }}
            >
              Manage connectors on claude.ai
            </a>
          </p>

          <div className="source-list">
            {src.sources
              .filter((s) => s.id !== "whatsapp")
              .map((s) => {
                const options = connectorNames.includes(s.connector ?? "")
                  ? connectorNames
                  : [s.connector ?? "", ...connectorNames];
                return (
                  <div key={s.id} className="source-row">
                    <div className="source-row-head">
                      <span className={`health ${s.health}`} />
                      <strong>{s.label}</strong>
                      <span className="muted small grow">{s.statusText}</span>
                    </div>
                    <select
                      value={s.connector ?? ""}
                      disabled={src.connectors === null}
                      onChange={(e) => changeConnector(s.id, e.target.value)}
                      aria-label={`${s.label} connector`}
                    >
                      {options.map((n) => (
                        <option key={n} value={n}>
                          {n}
                        </option>
                      ))}
                    </select>
                  </div>
                );
              })}

            <div className="source-row">
              <div className="source-row-head">
                <span className={`health ${src.files.length ? "ok" : "missing"}`} />
                <strong>WhatsApp</strong>
                <span className="muted small grow">
                  Exported chats (.txt). In WhatsApp: chat → ⋯ → Export chat → Without media.
                </span>
              </div>
              {src.files.length > 0 && (
                <ul className="file-list">
                  {src.files.map((f) => (
                    <li key={f.name}>
                      <span className="file-name" title={f.name}>
                        {f.name}
                      </span>
                      <span className="muted small">{formatSize(f.size)}</span>
                      <button className="icon-btn visible" aria-label={`Remove ${f.name}`} onClick={() => removeWhatsapp(f.name)}>
                        ×
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <button className="btn" onClick={attachWhatsapp}>
                Attach .txt export…
              </button>
            </div>
          </div>

          <div className="row">
            <button className="btn" onClick={src.refreshConnectors} disabled={src.connectors === null}>
              {src.connectors === null ? "Checking connectors…" : "Re-check connections"}
            </button>
            {src.connectorsError && <span className="status bad">{src.connectorsError}</span>}
          </div>
        </section>

        <section>
          <h3>Claude</h3>
          <p className="muted small">
            Pricy uses the Claude Code CLI installed on this computer and your existing Claude login.
          </p>
          <label>
            CLI path <span className="muted">(leave empty to detect automatically)</span>
            <input
              value={settings.claudePath}
              placeholder="/Users/you/.local/bin/claude"
              onChange={(e) => setSettings({ ...settings, claudePath: e.target.value })}
            />
          </label>
          <div className="row">
            <button className="btn" onClick={test} disabled={checking}>
              {checking ? "Checking…" : "Test"}
            </button>
            {status && (
              <span className={`status ${status.version ? "ok" : "bad"}`}>
                {status.version ? `${status.version} — ${status.path}` : status.error}
              </span>
            )}
          </div>
          <label>
            Model
            <select value={settings.model} onChange={(e) => setSettings({ ...settings, model: e.target.value })}>
              {MODELS.map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
        </section>

        <div className="row end">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={save}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
