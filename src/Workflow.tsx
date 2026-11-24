import { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Chat, ChatStatus, Message, Usage, WorkflowEvent, WorkflowStep } from "./api";

// ---------- status ----------

export const STATUS: Record<ChatStatus, { label: string; tone: "muted" | "accent" | "ok" | "warn"; hint: string }> = {
  new: { label: "New", tone: "muted", hint: "Describe the request and search." },
  answered: { label: "Price found", tone: "accent", hint: "Send it to Operations for approval, or mark it as not correct." },
  not_found: { label: "No price", tone: "warn", hint: "Draft a message to ask the supplier." },
  pending_approval: { label: "Waiting for approval", tone: "warn", hint: "Mark it approved or not correct once Operations replies." },
  approved: { label: "Approved", tone: "ok", hint: "Write the Supplier Notes for Sales." },
  supplier_draft: { label: "Supplier draft", tone: "accent", hint: "Send the draft to Operations so they can forward it to the supplier." },
  waiting_supplier: { label: "Waiting for supplier", tone: "warn", hint: "Paste the supplier's answer when Operations forwards it." },
  supplier_answered: { label: "Supplier answered", tone: "ok", hint: "Write the Supplier Notes for Sales." },
  final_ready: { label: "Notes ready", tone: "ok", hint: "Send the Supplier Notes to Sales (they copy them into TINT)." },
  completed: { label: "Completed", tone: "ok", hint: "The Supplier Notes were sent to Sales." },
};

export function StatusPill({ status }: { status: ChatStatus }) {
  const s = STATUS[status] ?? STATUS.new;
  return <span className={`status-pill ${s.tone}`}>{s.label}</span>;
}

// ---------- team ----------

export interface TeamMember {
  name: string;
  email: string;
}

/** "Anna Rossi <anna@x.com>" or "anna@x.com", one per line (commas also accepted). */
export function parseTeam(text: string): TeamMember[] {
  return text
    .split(/[\n,;]+/)
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => {
      const m = l.match(/^(.*?)\s*<([^>]+)>$/);
      const email = (m ? m[2] : l).trim();
      const name = (m?.[1] || email.split("@")[0]).trim();
      return { name, email };
    })
    .filter((p) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(p.email));
}

// ---------- what to send, per step ----------

export type SendKind = "approval" | "ops" | "sales";

const SEND_EVENT: Record<SendKind, WorkflowEvent> = {
  approval: "sent_for_approval",
  ops: "sent_to_ops",
  sales: "sent_to_sales",
};

const SEND_TITLE: Record<SendKind, string> = {
  approval: "Send for approval",
  ops: "Send to Operations",
  sales: "Send Supplier Notes to Sales",
};

function lastOf(messages: Message[], pred: (m: Message) => boolean) {
  for (let i = messages.length - 1; i >= 0; i--) if (pred(messages[i])) return messages[i];
  return undefined;
}

function plain(md: string) {
  return md
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/__(.+?)__/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/`([^`]+)`/g, "$1")
    .trim();
}

function draftEmail(kind: SendKind, chat: Chat, messages: Message[]) {
  const request = messages.find((m) => m.role === "user" && m.kind === "chat")?.content ?? chat.title;
  if (kind === "approval") {
    const answer = lastOf(messages, (m) => m.role === "assistant" && m.kind === "chat")?.content ?? "";
    return {
      subject: `Approval needed: ${chat.title}`,
      body: `Hi,\n\nPlease check this price before it goes to Sales.\n\nREQUEST\n${request}\n\nPROPOSED ANSWER\n${plain(answer)}\n\nReply "OK" to approve. If there are several options, tell me which price to communicate to the Sales team. If something is wrong, tell me what.\n\nSent from Pricy`,
    };
  }
  if (kind === "ops") {
    const draft = lastOf(messages, (m) => m.kind === "supplier_draft")?.content ?? "";
    return {
      subject: `Please send to supplier: ${chat.title}`,
      body: `Hi,\n\nWe don't have a confirmed price for this request. Could you send this message to the supplier and forward me their answer?\n\n-----\n${plain(draft)}\n-----\n\nSent from Pricy`,
    };
  }
  const final = lastOf(messages, (m) => m.kind === "final")?.content ?? "";
  return {
    subject: `Supplier Notes: ${chat.title}`,
    body: `Hi,\n\nHere are the Supplier Notes for this service. Please copy them into the Supplier Notes section in TINT.\n\n${plain(final)}\n\nSent from Pricy`,
  };
}

// ---------- workflow bar ----------

export function WorkflowBar({
  chat,
  messages,
  busy,
  onSend,
  onEvent,
  onStep,
  onRecordAnswer,
  onApprove,
}: {
  chat: Chat;
  messages: Message[];
  busy: boolean;
  onSend: (kind: SendKind) => void;
  onEvent: (e: WorkflowEvent, then?: WorkflowStep) => void;
  onStep: (s: WorkflowStep, supplierAnswer?: string) => void;
  onRecordAnswer: () => void;
  onApprove: () => void;
}) {
  const status = chat.status;
  const lastSupplierAnswer = () => {
    const ev = lastOf(messages, (m) => m.kind === "supplier_answered");
    return ev?.content.split("\n").filter((l) => l.startsWith("> ")).map((l) => l.slice(2)).join("\n");
  };

  const actions: { label: string; primary?: boolean; run: () => void }[] = [];
  switch (status) {
    case "answered":
      actions.push({ label: "Send for approval…", primary: true, run: () => onSend("approval") });
      actions.push({ label: "Not correct — ask supplier", run: () => onEvent("rejected", "draft_supplier") });
      break;
    case "not_found":
      actions.push({ label: "Draft supplier message", primary: true, run: () => onStep("draft_supplier") });
      break;
    case "pending_approval":
      actions.push({ label: "Approved…", primary: true, run: onApprove });
      actions.push({ label: "Not correct — ask supplier", run: () => onEvent("rejected", "draft_supplier") });
      actions.push({ label: "Send again…", run: () => onSend("approval") });
      break;
    case "approved":
      actions.push({ label: "Write Supplier Notes", primary: true, run: () => onStep("final_from_approval") });
      break;
    case "supplier_draft":
      actions.push({ label: "Send to Operations…", primary: true, run: () => onSend("ops") });
      actions.push({ label: "Redraft", run: () => onStep("draft_supplier") });
      actions.push({ label: "Record supplier answer…", run: onRecordAnswer });
      break;
    case "waiting_supplier":
      actions.push({ label: "Record supplier answer…", primary: true, run: onRecordAnswer });
      actions.push({ label: "Send again…", run: () => onSend("ops") });
      break;
    case "supplier_answered":
      actions.push({
        label: "Write Supplier Notes",
        primary: true,
        run: () => onStep("final_from_supplier", lastSupplierAnswer()),
      });
      break;
    case "final_ready":
      actions.push({ label: "Send to Sales…", primary: true, run: () => onSend("sales") });
      break;
    case "completed":
      actions.push({ label: "Send again…", run: () => onSend("sales") });
      break;
  }

  if (status === "new") return null;
  return (
    <div className="workflow-bar">
      <StatusPill status={status} />
      <span className="workflow-hint muted">{STATUS[status]?.hint}</span>
      <div className="workflow-actions">
        {actions.map((a) => (
          <button key={a.label} className={`btn small ${a.primary ? "primary" : ""}`} disabled={busy} onClick={a.run}>
            {a.label}
          </button>
        ))}
      </div>
    </div>
  );
}

// ---------- send dialog (mail) ----------

export function SendDialog({
  kind,
  chat,
  messages,
  team,
  onClose,
  onSent,
  onOpenSettings,
}: {
  kind: SendKind;
  chat: Chat;
  messages: Message[];
  team: TeamMember[];
  onClose: () => void;
  onSent: (event: WorkflowEvent, recipients: string[]) => void;
  onOpenSettings: () => void;
}) {
  const initial = useMemo(() => draftEmail(kind, chat, messages), [kind, chat, messages]);
  const [subject, setSubject] = useState(initial.subject);
  const [body, setBody] = useState(initial.body);
  const [picked, setPicked] = useState<Set<string>>(new Set(team.length === 1 ? [team[0].email] : []));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  function toggle(email: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(email)) next.delete(email);
      else next.add(email);
      return next;
    });
  }

  async function send() {
    const to = [...picked];
    const url = `mailto:${to.map(encodeURIComponent).join(",")}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
    await openUrl(url);
    onSent(SEND_EVENT[kind], to);
  }

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog wide" onMouseDown={(e) => e.stopPropagation()}>
        <h2>{SEND_TITLE[kind]}</h2>

        <h3>To</h3>
        {team.length === 0 ? (
          <p className="muted small">
            No team members yet.{" "}
            <button className="link-btn" onClick={onOpenSettings}>
              Add their emails in Settings
            </button>
            .
          </p>
        ) : (
          <div className="recipients">
            {team.map((p) => (
              <label key={p.email} className={`source-chip ${picked.has(p.email) ? "on" : ""}`} title={p.email}>
                <input type="checkbox" checked={picked.has(p.email)} onChange={() => toggle(p.email)} />
                {p.name}
                <span className="muted small">{p.email}</span>
              </label>
            ))}
          </div>
        )}

        <label>
          Subject
          <input value={subject} onChange={(e) => setSubject(e.target.value)} />
        </label>
        <label>
          Message
          <textarea className="email-body" value={body} onChange={(e) => setBody(e.target.value)} rows={12} />
        </label>
        <p className="muted small">Opens your mail app with this message. Send it from there.</p>

        <div className="row end">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={send} disabled={picked.size === 0}>
            Open in Mail
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------- approval (Operations) ----------

/** Emails the last message of `kind` was sent to, e.g. who was asked for approval. */
export function lastRecipients(messages: Message[], kind: "sent_for_approval" | "sent_to_ops"): string[] {
  const ev = lastOf(messages, (m) => m.role === "event" && m.kind === kind);
  const list = ev?.content.split(/ to |: /).pop() ?? "";
  return list.split(",").map((e) => e.trim()).filter(Boolean);
}

function PersonPicker({
  label,
  team,
  value,
  onChange,
}: {
  label: string;
  team: TeamMember[];
  value: string;
  onChange: (v: string) => void;
}) {
  const listId = "team-names";
  return (
    <label>
      {label}
      <input list={listId} value={value} placeholder="Name" onChange={(e) => onChange(e.target.value)} />
      <datalist id={listId}>
        {team.map((p) => (
          <option key={p.email} value={p.name}>
            {p.email}
          </option>
        ))}
      </datalist>
    </label>
  );
}

function suggestedName(team: TeamMember[], emails: string[]) {
  return emails.length === 1 ? (team.find((p) => p.email === emails[0])?.name ?? emails[0]) : "";
}

export function ApproveDialog({
  team,
  suggested,
  onClose,
  onApprove,
}: {
  team: TeamMember[];
  suggested: string[];
  onClose: () => void;
  onApprove: (approver: string, note: string) => void;
}) {
  const [approver, setApprover] = useState(suggestedName(team, suggested));
  const [note, setNote] = useState("");
  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Price approved</h2>
        <PersonPicker label="Approved by (Operations)" team={team} value={approver} onChange={setApprover} />
        <label>
          Chosen option / note <span className="muted">(if there were several prices)</span>
          <textarea
            className="email-body"
            rows={3}
            value={note}
            placeholder="e.g. Option 2 — Sud Transfer, €620 Net Price"
            onChange={(e) => setNote(e.target.value)}
          />
        </label>
        <p className="muted small">Claude then writes the Supplier Notes for Sales.</p>
        <div className="row end">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={!approver.trim()} onClick={() => onApprove(approver.trim(), note.trim())}>
            Approve and write notes
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------- supplier answer dialog ----------

export function SupplierAnswerDialog({
  team,
  suggested,
  onClose,
  onSave,
}: {
  team: TeamMember[];
  suggested: string[];
  onClose: () => void;
  onSave: (text: string, approver: string) => void;
}) {
  const [text, setText] = useState("");
  const [approver, setApprover] = useState(suggestedName(team, suggested));
  return (
    <div className="overlay" onMouseDown={onClose}>
      <div className="dialog wide" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Record supplier answer</h2>
        <p className="muted small">
          Paste the supplier's reply as Operations forwarded it. It is saved to <strong>Saved answers</strong> so
          future searches find it, then Claude writes the Supplier Notes for Sales.
        </p>
        <textarea
          className="email-body"
          autoFocus
          rows={10}
          value={text}
          placeholder="e.g. Sedan Matera → FCO, 18 May 2027, 2 pax: €640 net, 4h40, luggage included."
          onChange={(e) => setText(e.target.value)}
        />
        <PersonPicker label="Forwarded / approved by (Operations)" team={team} value={approver} onChange={setApprover} />
        <div className="row end">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={!text.trim()} onClick={() => onSave(text, approver.trim())}>
            Save and write Supplier Notes
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------- token usage ----------

const fmt = (n: number) => n.toLocaleString("en-US");

export function UsageLine({ usage }: { usage: Usage }) {
  const total = usage.inputTokens + usage.cacheCreationTokens + usage.cacheReadTokens + usage.outputTokens;
  const detail = [
    `Input: ${fmt(usage.inputTokens)}`,
    `Cache write: ${fmt(usage.cacheCreationTokens)}`,
    `Cache read: ${fmt(usage.cacheReadTokens)}`,
    `Output: ${fmt(usage.outputTokens)}`,
    `Model calls: ${usage.numTurns}`,
    usage.models.length ? `Model: ${usage.models.join(", ")}` : "",
    "Cost is the API-equivalent estimate reported by Claude Code.",
  ]
    .filter(Boolean)
    .join("\n");
  return (
    <span className="usage muted" title={detail}>
      {fmt(total)} tokens ({fmt(usage.inputTokens + usage.cacheCreationTokens)} in · {fmt(usage.cacheReadTokens)} cached ·{" "}
      {fmt(usage.outputTokens)} out) · ${usage.costUsd.toFixed(3)} · {(usage.durationMs / 1000).toFixed(1)}s
    </span>
  );
}

export function totalUsage(messages: Message[]) {
  let tokens = 0;
  let cost = 0;
  for (const m of messages) {
    const u = m.usage;
    if (!u) continue;
    tokens += u.inputTokens + u.cacheCreationTokens + u.cacheReadTokens + u.outputTokens;
    cost += u.costUsd;
  }
  return { tokens, cost };
}
