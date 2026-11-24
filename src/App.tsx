import { useCallback, useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, onStreamEvent, onTurnFinished, type Chat, type ClaudeStatus, type Message } from "./api";
import { SettingsDialog } from "./SettingsDialog";
import "./App.css";

interface Streaming {
  text: string;
  tools: string[];
}

const EXAMPLES = [
  "2 pax, Matera to Rome, 18 May 2027 — private transfer. What's the price and travel time?",
  "Did we already quote a transfer from Naples airport to Positano for 4 pax?",
  "Latest supplier price for a minivan transfer Rome FCO → city centre",
];

export default function App() {
  const [chats, setChats] = useState<Chat[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState<Record<string, Streaming>>({});
  const [running, setRunning] = useState<Set<string>>(new Set());
  const [input, setInput] = useState("");
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  const activeRef = useRef<string | null>(null);
  activeRef.current = activeId;
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const refreshChats = useCallback(() => api.listChats().then(setChats).catch(showError), []);
  const refreshStatus = useCallback(() => api.checkClaude().then(setStatus).catch(showError), []);

  function showError(e: unknown) {
    setToast(String(e));
    setTimeout(() => setToast(null), 6000);
  }

  // Initial load + stream listeners.
  useEffect(() => {
    refreshChats();
    refreshStatus();
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
  }, [refreshChats, refreshStatus]);

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
      const userMsg = await api.sendMessage(chatId, text);
      setInput("");
      setRunning((prev) => new Set(prev).add(chatId!));
      setStreaming((prev) => ({ ...prev, [chatId!]: { text: "", tools: [] } }));
      if (chatId === activeRef.current || !activeId) {
        setMessages((prev) => [...prev, userMsg]);
      }
      refreshChats();
    } catch (e) {
      showError(e);
    }
  }

  async function newChat() {
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

  const isRunning = activeId ? running.has(activeId) : false;
  const live = activeId ? streaming[activeId] : undefined;
  const claudeMissing = status && !status.version;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="logo">P</span> Pricy
        </div>
        <button className="new-chat" onClick={newChat}>
          + New search
        </button>
        <nav className="chat-list">
          {chats.length === 0 && <p className="muted small pad">No history yet</p>}
          {chats.map((c) => (
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
        </nav>
        <div className="sidebar-footer">
          <button className="link-btn" onClick={() => setShowSettings(true)}>
            ⚙ Settings
          </button>
          <span className={`status ${status?.version ? "ok" : "bad"}`} title={status?.path ?? ""}>
            {status === null ? "Checking Claude…" : status.version ? "Claude connected" : "Claude not found"}
          </span>
        </div>
      </aside>

      <main className="main">
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
              <h1>Find a transfer price</h1>
              <p className="muted">Describe the transfer (route, date, pax, service type). Pricy looks for an existing price in the shared data.</p>
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
                <MessageView key={m.id} role={m.role} content={m.content} />
              ))}
              {isRunning && live && (
                <div className="msg assistant">
                  {live.tools.length > 0 && (
                    <div className="tools">
                      {live.tools.map((t, i) => (
                        <span key={i} className="tool-chip">
                          {prettyTool(t)}
                        </span>
                      ))}
                    </div>
                  )}
                  {live.text ? <Markdown text={live.text} /> : <div className="typing">Searching…</div>}
                </div>
              )}
              <div ref={bottomRef} />
            </div>
          )}
        </div>

        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            if (!isRunning) send(input);
          }}
        >
          <textarea
            ref={inputRef}
            value={input}
            placeholder="Ask for a price… (Enter to send, Shift+Enter for a new line)"
            rows={2}
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
            <button type="submit" className="btn primary" disabled={!input.trim() || !!claudeMissing}>
              Send
            </button>
          )}
        </form>
      </main>

      {showSettings && (
        <SettingsDialog
          onClose={() => {
            setShowSettings(false);
            refreshStatus();
          }}
          onError={showError}
        />
      )}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

function MessageView({ role, content }: { role: Message["role"]; content: string }) {
  if (role === "user") return <div className="msg user">{content}</div>;
  return (
    <div className={`msg ${role}`}>
      <Markdown text={content} />
    </div>
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

function prettyTool(name: string) {
  // mcp__server__tool → "server: tool"
  const m = name.match(/^mcp__(.+?)__(.+)$/);
  return m ? `${m[1]}: ${m[2]}` : name;
}
