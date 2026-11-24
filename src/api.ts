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
  sendMessage: (chatId: string, text: string) => invoke<Message>("send_message", { chatId, text }),
  stopMessage: (chatId: string) => invoke<void>("stop_message", { chatId }),
  runningChats: () => invoke<string[]>("running_chats"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (s: Settings) => invoke<void>("save_settings", { ...s }),
  checkClaude: () => invoke<ClaudeStatus>("check_claude"),
};

export const onStreamEvent = (cb: (e: StreamEvent) => void): Promise<UnlistenFn> =>
  listen<StreamEvent>("claude://event", (e) => cb(e.payload));

export const onTurnFinished = (cb: (e: TurnFinished) => void): Promise<UnlistenFn> =>
  listen<TurnFinished>("claude://done", (e) => cb(e.payload));
