import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Chat {
  id: string;
  title: string;
  claudeSessionId: string | null;
  createdAt: number;
  updatedAt: number;
  status: ChatStatus;
}

/** Workflow state of a request (see architecture/WORKFLOW.md). */
export type ChatStatus =
  | "new"
  | "answered"
  | "not_found"
  | "pending_approval"
  | "approved"
  | "supplier_draft"
  | "waiting_supplier"
  | "supplier_answered"
  | "final_ready"
  | "completed";

export type WorkflowEvent = "sent_for_approval" | "approved" | "rejected" | "sent_to_ops" | "sent_to_sales";
export type WorkflowStep = "draft_supplier" | "final_from_approval" | "final_from_supplier";

export interface Usage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  /** API-equivalent cost reported by Claude Code. */
  costUsd: number;
  durationMs: number;
  numTurns: number;
  models: string[];
}

export type Role = "user" | "assistant" | "error" | "event";
export type MessageKind = "chat" | "action" | "supplier_draft" | "final" | string;

export interface Message {
  id: number;
  chatId: string;
  role: Role;
  kind: MessageKind;
  content: string;
  createdAt: number;
  /** Source ids searched for this message (user messages). */
  sources: string[];
  usage: Usage | null;
}

export type SourceId = "tint" | "slack" | "whatsapp" | "saved" | "drive";

export interface SourceInfo {
  id: SourceId;
  label: string;
  /** Default claude.ai connector name; null for local sources (WhatsApp). */
  defaultConnector: string | null;
  hint: string;
  /** Connector actually used (default or overridden in Settings). */
  connector: string | null;
  active: boolean;
  /** File count for local sources (WhatsApp, Saved answers). */
  files: number | null;
}

export interface Connector {
  name: string;
  status: string;
  connected: boolean;
}

export interface SourceFile {
  name: string;
  size: number;
  modifiedAt: number;
}

export interface Settings {
  claudePath: string;
  model: string;
  /** One team member per line: "Name <email>" or "email". */
  team: string;
}

export interface ClaudeStatus {
  path: string | null;
  version: string | null;
  error: string | null;
}

export type StreamEvent =
  | { kind: "delta"; chatId: string; text: string }
  | { kind: "tool"; chatId: string; name: string; input: unknown };

export interface TurnFinished {
  chatId: string;
  message: Message | null;
  cancelled: boolean;
}

export const api = {
  listChats: () => invoke<Chat[]>("list_chats"),
  createChat: () => invoke<Chat>("create_chat"),
  renameChat: (chatId: string, title: string) => invoke<void>("rename_chat", { chatId, title }),
  deleteChat: (chatId: string) => invoke<void>("delete_chat", { chatId }),
  getMessages: (chatId: string) => invoke<Message[]>("get_messages", { chatId }),
  sendMessage: (chatId: string, text: string, sources: SourceId[]) =>
    invoke<Message>("send_message", { chatId, text, sources }),
  stopMessage: (chatId: string) => invoke<void>("stop_message", { chatId }),
  runningChats: () => invoke<string[]>("running_chats"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (s: Settings) => invoke<void>("save_settings", { ...s }),
  checkClaude: () => invoke<ClaudeStatus>("check_claude"),
  listSources: () => invoke<SourceInfo[]>("list_sources"),
  setActiveSources: (sources: SourceId[]) => invoke<void>("set_active_sources", { sources }),
  setConnector: (sourceId: SourceId, connector: string) => invoke<void>("set_connector", { sourceId, connector }),
  listConnectors: () => invoke<Connector[]>("list_connectors"),
  listWhatsappFiles: () => invoke<SourceFile[]>("list_whatsapp_files"),
  addWhatsappFiles: (paths: string[]) => invoke<string[]>("add_whatsapp_files", { paths }),
  removeWhatsappFile: (name: string) => invoke<void>("remove_whatsapp_file", { name }),
  workflowEvent: (chatId: string, event: WorkflowEvent, recipients: string[] = []) =>
    invoke<Message[]>("workflow_event", { chatId, event, recipients }),
  runStep: (chatId: string, step: WorkflowStep, supplierAnswer?: string) =>
    invoke<Message[]>("run_step", { chatId, step, supplierAnswer: supplierAnswer ?? null }),
};

export const onStreamEvent = (cb: (e: StreamEvent) => void): Promise<UnlistenFn> =>
  listen<StreamEvent>("claude://event", (e) => cb(e.payload));

export const onTurnFinished = (cb: (e: TurnFinished) => void): Promise<UnlistenFn> =>
  listen<TurnFinished>("claude://done", (e) => cb(e.payload));
