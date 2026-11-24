import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Chat {
  id: string;
  title: string;
  claudeSessionId: string | null;
  createdAt: number;
  updatedAt: number;
}

export type Role = "user" | "assistant" | "error";

export interface Message {
  id: number;
  chatId: string;
  role: Role;
  content: string;
  createdAt: number;
  /** Source ids searched for this message (user messages). */
  sources: string[];
}

export type SourceId = "tint" | "slack" | "whatsapp" | "drive";

export interface SourceInfo {
  id: SourceId;
  label: string;
  /** Default claude.ai connector name; null for local sources (WhatsApp). */
  defaultConnector: string | null;
  hint: string;
  /** Connector actually used (default or overridden in Settings). */
  connector: string | null;
  active: boolean;
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
};

export const onStreamEvent = (cb: (e: StreamEvent) => void): Promise<UnlistenFn> =>
  listen<StreamEvent>("claude://event", (e) => cb(e.payload));

export const onTurnFinished = (cb: (e: TurnFinished) => void): Promise<UnlistenFn> =>
  listen<TurnFinished>("claude://done", (e) => cb(e.payload));
