import { useEffect, useState } from "react";
import { api, type ClaudeStatus, type Settings } from "./api";

const MODELS = [
  { value: "", label: "Default (Claude Code setting)" },
  { value: "sonnet", label: "Sonnet — balanced" },
  { value: "opus", label: "Opus — most capable" },
  { value: "haiku", label: "Haiku — fastest" },
];

export function SettingsDialog({ onClose, onError }: { onClose: () => void; onError: (e: unknown) => void }) {
  const [settings, setSettings] = useState<Settings>({ claudePath: "", model: "" });
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    api.getSettings().then(setSettings).catch(onError);
    api.checkClaude().then(setStatus).catch(onError);
  }, []);

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

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Settings</h2>

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

        <section>
          <h3>Data sources</h3>
          <p className="muted small">TINT, Slack, WhatsApp exports and Google Drive will be connected here.</p>
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
