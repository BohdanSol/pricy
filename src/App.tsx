import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  api,
  onStreamEvent,
  onTurnFinished,
  type Chat,
  type ClaudeStatus,
  type Message,
  type WorkflowEvent,
  type WorkflowStep,
} from "./api";
import { SettingsDialog } from "./SettingsDialog";
import {
  parseTeam,
  SendDialog,
  StatusPill,
  SupplierAnswerDialog,
  totalUsage,
  UsageLine,
  WorkflowBar,
  type SendKind,
  type TeamMember,
} from "./Workflow";
import { useSources, type SourceState } from "./useSources";
import logo from "./assets/logo.png";
import "./App.css";

interface Streaming {
  text: string;
  tools: string[];
}

const EXAMPLES = [
  "2 pax, Matera to Rome, 18 May 2027 — private transfer. What's the price and travel time?",
  "Private walking tour of the Vatican Museums for 6 pax in June — what did we quote last time?",
  "Latest supplier price for an Amalfi Coast boat tour, 4 pax",
];

const isMac = navigator.userAgent.includes("Mac");
const MOD = isMac ? "⌘" : "Ctrl+";

export default function App() {
  const [chats, setChats] = useState<Chat[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState<Record<string, Streaming>>({});
  const [running, setRunning] = useState<Set<string>>(new Set());
  const [input, setInput] = useState("");
  const [filter, setFilter] = useState("");
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [team, setTeam] = useState<TeamMember[]>([]);
  const [sendKind, setSendKind] = useState<SendKind | null>(null);
  const [recordingAnswer, setRecordingAnswer] = useState(false);

  const activeRef = useRef<string | null>(null);
  activeRef.current = activeId;
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);

  const showError = useCallback((e: unknown) => {
    setToast(String(e));
    setTimeout(() => setToast(null), 6000);
  }, []);
  const refreshChats = useCallback(() => api.listChats().then(setChats).catch(showError), [showError]);
  const refreshStatus = useCallback(() => api.checkClaude().then(setStatus).catch(showError), [showError]);
  const refreshTeam = useCallback(
    () => api.getSettings().then((s) => setTeam(parseTeam(s.team))).catch(showError),
    [showError],
  );
  const src = useSources(showError);

  // Initial load + stream listeners.
  useEffect(() => {
    refreshChats();
    refreshStatus();
    refreshTeam();
    api.runningChats().then((ids) => setRunning(new Set(ids)));

    const unlistenEvent = onStreamEvent((e) => {
      setStreaming((prev) => {
        const cur = prev[e.chatId] ?? { text: "", tools: [] };
        const next =
          e.kind === "delta"
            ? { ...cur, text: cur.text + e.text }
            : { ...cur, tools: [...cur.tools, e.name] };
        return { ...prev, [e.chatId]: next };
      });
    });

    const unlistenDone = onTurnFinished((e) => {
      setStreaming(({ [e.chatId]: _, ...rest }) => rest);
      setRunning((prev) => {
        const next = new Set(prev);
        next.delete(e.chatId);
        return next;
      });
      if (e.message && e.chatId === activeRef.current) {
        setMessages((prev) => [...prev, e.message!]);
      }
      refreshChats();
    });

    return () => {
      unlistenEvent.then((f) => f());
      unlistenDone.then((f) => f());
    };
  }, [refreshChats, refreshStatus, refreshTeam]);

  // Keyboard shortcuts: ⌘N new search, ⌘K search history, ⌘, settings.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(isMac ? e.metaKey : e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "n") {
        e.preventDefault();
        newChat();
      } else if (key === "k") {
        e.preventDefault();
        filterRef.current?.focus();
      } else if (key === ",") {
        e.preventDefault();
        setShowSettings(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Load messages when switching chats.
  useEffect(() => {
    if (!activeId) {
      setMessages([]);
      return;
    }
    api.getMessages(activeId).then(setMessages).catch(showError);
    inputRef.current?.focus();
  }, [activeId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [messages, streaming, activeId]);

  async function send(text: string) {
    text = text.trim();
    if (!text) return;
    try {
      let chatId = activeId;
      if (!chatId) {
        const chat = await api.createChat();
        chatId = chat.id;
        setActiveId(chat.id);
      }
      const userMsg = await api.sendMessage(chatId, text, [...src.active]);
      setInput("");
      markRunning(chatId);
      if (chatId === activeRef.current || !activeId) {
        setMessages((prev) => [...prev, userMsg]);
      }
      refreshChats();
    } catch (e) {
      showError(e);
    }
  }

  function markRunning(chatId: string) {
    setRunning((prev) => new Set(prev).add(chatId));
    setStreaming((prev) => ({ ...prev, [chatId]: { text: "", tools: [] } }));
  }

  function appendIfActive(chatId: string, added: Message[]) {
    if (chatId === activeRef.current) setMessages((prev) => [...prev, ...added]);
  }

  /** A step Claude performs (supplier draft, final answer). */
  async function runStep(step: WorkflowStep, supplierAnswer?: string) {
    if (!activeId) return;
    const chatId = activeId;
    try {
      const added = await api.runStep(chatId, step, supplierAnswer);
      appendIfActive(chatId, added);
      markRunning(chatId);
    } catch (e) {
      showError(e);
    } finally {
      refreshChats();
      src.refreshSources();
    }
  }

  /** A step done by a person (sent an email, approved…), optionally followed by a Claude step. */
  async function workflowEvent(event: WorkflowEvent, recipients: string[] = [], then?: WorkflowStep) {
    if (!activeId) return;
    const chatId = activeId;
    try {
      appendIfActive(chatId, await api.workflowEvent(chatId, event, recipients));
      await refreshChats();
      if (then) await runStep(then);
    } catch (e) {
      showError(e);
    }
  }

  function newChat() {
    setActiveId(null);
    setInput("");
    inputRef.current?.focus();
  }

  async function deleteChat(chat: Chat) {
    if (!confirm(`Delete "${chat.title}"?`)) return;
    await api.deleteChat(chat.id).catch(showError);
    if (chat.id === activeId) setActiveId(null);
    refreshChats();
  }

  async function renameChat(chat: Chat) {
    const title = prompt("Rename chat", chat.title);
    if (!title?.trim()) return;
    await api.renameChat(chat.id, title).catch(showError);
    refreshChats();
  }

  const groups = useMemo(() => groupChats(chats, filter), [chats, filter]);
  const activeChat = chats.find((c) => c.id === activeId);
  const isRunning = activeId ? running.has(activeId) : false;
  const live = activeId ? streaming[activeId] : undefined;
  const claudeMissing = status && !status.version;
  const totals = useMemo(() => totalUsage(messages), [messages]);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <img src={logo} alt="Pricy" />
        </div>
        <button className="new-chat" onClick={newChat}>
          <span>+ New search</span>
          <kbd>{MOD}N</kbd>
        </button>
        <div className="history-search">
          <input
            ref={filterRef}
            value={filter}
            placeholder={`Search history  ${MOD}K`}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && setFilter("")}
          />
        </div>
        <nav className="chat-list">
          {chats.length === 0 && <p className="muted small pad">No history yet</p>}
          {chats.length > 0 && groups.length === 0 && <p className="muted small pad">Nothing matches “{filter}”</p>}
          {groups.map(([label, items]) => (
            <div key={label} className="chat-group">
              <div className="group-label">{label}</div>
              {items.map((c) => (
                <div
                  key={c.id}
                  className={`chat-item ${c.id === activeId ? "active" : ""}`}
                  onClick={() => setActiveId(c.id)}
                  onDoubleClick={() => renameChat(c)}
                  title={c.title}
                >
                  <span className="chat-title">
                    {running.has(c.id) && <span className="dot" />}
                    {c.title}
                  </span>
                  {c.status !== "new" && <StatusPill status={c.status} />}
                  <button
                    className="icon-btn"
                    aria-label="Delete chat"
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteChat(c);
                    }}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          ))}
        </nav>
        <div className="sidebar-footer">
          <span className={`status ${status?.version ? "ok" : "bad"}`} title={status?.path ?? ""}>
            <span className="status-dot" />
            {status === null ? "Checking Claude…" : status.version ? "Claude connected" : "Claude not found"}
          </span>
          <button className="icon-text-btn" onClick={() => setShowSettings(true)} title={`Settings (${MOD},)`}>
            ⚙ Settings
          </button>
        </div>
      </aside>

      <main className="main">
        {activeChat && (
          <header className="chat-header">
            <h2 title={activeChat.title} onDoubleClick={() => renameChat(activeChat)}>
              {activeChat.title}
            </h2>
            <StatusPill status={activeChat.status} />
            {totals.tokens > 0 && (
              <span className="muted small" title="Total for this request (API-equivalent cost reported by Claude Code)">
                {totals.tokens.toLocaleString("en-US")} tokens · ${totals.cost.toFixed(3)}
              </span>
            )}
            <span className="muted small">{formatDate(activeChat.createdAt)}</span>
          </header>
        )}

        {claudeMissing && (
          <div className="banner">
            Claude Code CLI wasn't found on this computer. Install it and sign in, or set its path in{" "}
            <button className="link-btn" onClick={() => setShowSettings(true)}>
              Settings
            </button>
            .
          </div>
        )}

        <div className="messages">
          {messages.length === 0 && !live ? (
            <div className="empty">
              <div className="empty-dot" />
              <h1>Find a price for a transfer or tour</h1>
              <p className="muted">
                Describe the service (transfer or tour, date, pax, places, service type). Pricy looks for an existing price in the
                shared data.
              </p>
              <div className="examples">
                {EXAMPLES.map((ex) => (
                  <button key={ex} onClick={() => send(ex)} disabled={!!claudeMissing}>
                    {ex}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <div className="thread">
              {messages.map((m) => (
                <MessageView key={m.id} message={m} sources={src.sources} />
              ))}
              {isRunning && live && (
                <div className="msg assistant">
                  {live.tools.length > 0 && (
                    <div className="tools">
                      {live.tools.map((t, i) => (
                        <span key={i} className="tool-chip">
                          {prettyTool(t, src.sources)}
                        </span>
                      ))}
                    </div>
                  )}
                  {live.text ? (
                    <Markdown text={live.text.replace(/\[\[pricy:(not_)?found\]\]/g, "")} />
                  ) : (
                    <div className="typing">
                      <span />
                      <span />
                      <span />
                    </div>
                  )}
                </div>
              )}
              <div ref={bottomRef} />
            </div>
          )}
        </div>

        {activeChat && (
          <WorkflowBar
            chat={activeChat}
            messages={messages}
            busy={isRunning}
            onSend={setSendKind}
            onEvent={(e, then) => workflowEvent(e, [], then)}
            onStep={runStep}
            onRecordAnswer={() => setRecordingAnswer(true)}
          />
        )}

        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            if (!isRunning) send(input);
          }}
        >
          <SourceBar
            sources={src.sources}
            active={src.active}
            onToggle={src.toggle}
            onOpenSettings={() => setShowSettings(true)}
          />
          <div className="composer-box">
            <textarea
              ref={inputRef}
              value={input}
              placeholder="Describe the transfer or tour request…"
              rows={1}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  if (!isRunning) send(input);
                }
              }}
            />
            {isRunning ? (
              <button type="button" className="btn stop" onClick={() => api.stopMessage(activeId!)}>
                Stop
              </button>
            ) : (
              <button
              type="submit"
              className="btn primary"
              disabled={!input.trim() || !!claudeMissing || src.active.size === 0}
              title={src.active.size === 0 ? "Select at least one source to search" : undefined}
            >
                Send
              </button>
            )}
          </div>
          <div className="composer-hint muted">Enter to send · Shift+Enter for a new line</div>
        </form>
      </main>

      {showSettings && (
        <SettingsDialog
          sources={src}
          onClose={() => {
            setShowSettings(false);
            refreshStatus();
            refreshTeam();
            src.refreshSources();
          }}
          onError={showError}
        />
      )}
      {sendKind && activeChat && (
        <SendDialog
          kind={sendKind}
          chat={activeChat}
          messages={messages}
          team={team}
          onClose={() => setSendKind(null)}
          onOpenSettings={() => {
            setSendKind(null);
            setShowSettings(true);
          }}
          onSent={(event, to) => {
            setSendKind(null);
            workflowEvent(event, to);
          }}
        />
      )}
      {recordingAnswer && (
        <SupplierAnswerDialog
          onClose={() => setRecordingAnswer(false)}
          onSave={(text) => {
            setRecordingAnswer(false);
            runStep("final_from_supplier", text);
          }}
        />
      )}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

function SourceBar({
  sources,
  active,
  onToggle,
  onOpenSettings,
}: {
  sources: SourceState[];
  active: Set<string>;
  onToggle: (id: SourceState["id"]) => void;
  onOpenSettings: () => void;
}) {
  return (
    <div className="source-bar" role="group" aria-label="Sources to search">
      <span className="source-bar-label muted">Search in</span>
      {sources.map((s) => {
        const checked = active.has(s.id);
        const unavailable = s.files !== null && (s.health === "missing" || s.health === "empty");
        return (
          <label
            key={s.id}
            className={`source-chip ${checked ? "on" : ""} ${unavailable ? "unavailable" : ""}`}
            title={`${s.hint}\n\n${s.statusText}`}
            onClick={(e) => {
              if (unavailable) {
                e.preventDefault();
                if (s.id === "whatsapp") onOpenSettings();
              }
            }}
          >
            <input type="checkbox" checked={checked && !unavailable} onChange={() => onToggle(s.id)} />
            <span className={`health ${s.health}`} />
            {s.label}
            {unavailable && <span className="chip-note">{s.id === "whatsapp" ? "attach" : "empty"}</span>}
          </label>
        );
      })}
    </div>
  );
}

const KIND_TITLE: Record<string, string> = {
  supplier_draft: "Message to the supplier",
  final: "Final answer for Sales",
};

function MessageView({ message, sources }: { message: Message; sources: SourceState[] }) {
  if (message.role === "event") {
    return (
      <div className="event-row">
        <span className="event-dot" />
        <div className="event-body">
          <Markdown text={message.content} />
        </div>
        <span className="muted small">{formatTime(message.createdAt)}</span>
      </div>
    );
  }
  if (message.role === "user" && message.kind === "action") {
    return (
      <div className="event-row action">
        <span className="event-dot" />
        <div className="event-body">{message.content}</div>
        <span className="muted small">{formatTime(message.createdAt)}</span>
      </div>
    );
  }
  if (message.role === "user") {
    const labels = message.sources.map((id) => sources.find((s) => s.id === id)?.label ?? id);
    return (
      <div className="user-wrap">
        <div className="msg user">{message.content}</div>
        {labels.length > 0 && <div className="searched muted">Searched: {labels.join(" · ")}</div>}
      </div>
    );
  }
  const cardTitle = KIND_TITLE[message.kind];
  return (
    <div className={`msg ${message.role} ${cardTitle ? `card ${message.kind}` : ""}`}>
      {cardTitle && <div className="card-title">{cardTitle}</div>}
      <Markdown text={message.content} />
      <div className="msg-meta">
        {message.role === "assistant" && <CopyButton text={message.content} />}
        {message.usage && <UsageLine usage={message.usage} />}
        <span className="muted small">{formatTime(message.createdAt)}</span>
      </div>
    </div>
  );
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="mini-btn"
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    >
      {copied ? "✓ Copied" : "Copy"}
    </button>
  );
}

function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(e) => {
                e.preventDefault();
                if (href) openUrl(href);
              }}
            >
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

/** "mcp__claude_ai_Google_Drive__search_files" → "Drive: search files", "Grep" → "WhatsApp: grep". */
function prettyTool(name: string, sources: SourceState[]) {
  if (name === "ToolSearch") return "Loading tools";
  if (["Grep", "Read", "Glob"].includes(name)) return `Local files: ${name.toLowerCase()}`;
  const m = name.match(/^mcp__(.+?)__(.+)$/);
  if (!m) return name;
  const norm = (c: string) => c.replace(/[^A-Za-z0-9_-]/g, "_");
  const source = sources.find((s) => s.connector && norm(s.connector) === m[1]);
  return `${source?.label ?? m[1]}: ${m[2].replace(/_/g, " ")}`;
}

function groupChats(chats: Chat[], filter: string): [string, Chat[]][] {
  const q = filter.trim().toLowerCase();
  const startOfDay = new Date().setHours(0, 0, 0, 0);
  const DAY = 86_400_000;
  const buckets: [string, (t: number) => boolean][] = [
    ["Today", (t) => t >= startOfDay],
    ["Yesterday", (t) => t >= startOfDay - DAY],
    ["Previous 7 days", (t) => t >= startOfDay - 7 * DAY],
    ["Previous 30 days", (t) => t >= startOfDay - 30 * DAY],
    ["Older", () => true],
  ];
  const out = new Map<string, Chat[]>();
  for (const c of chats) {
    if (q && !c.title.toLowerCase().includes(q)) continue;
    const label = buckets.find(([, test]) => test(c.updatedAt))![0];
    out.set(label, [...(out.get(label) ?? []), c]);
  }
  return [...out.entries()];
}

function formatDate(ms: number) {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function formatTime(ms: number) {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}
