import { useCallback, useEffect, useState } from "react";
import { api, type Connector, type SourceFile, type SourceId, type SourceInfo } from "./api";

export type SourceHealth = "ok" | "warn" | "missing" | "empty" | "checking";

export interface SourceState extends SourceInfo {
  health: SourceHealth;
  /** Human-readable status for tooltips and Settings. */
  statusText: string;
}

/** Data sources, their connection status and the WhatsApp files, shared by the app and Settings. */
export function useSources(onError: (e: unknown) => void) {
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [active, setActive] = useState<Set<SourceId>>(new Set());
  const [connectors, setConnectors] = useState<Connector[] | null>(null);
  const [connectorsError, setConnectorsError] = useState<string | null>(null);
  const [files, setFiles] = useState<SourceFile[]>([]);

  const refreshSources = useCallback(async () => {
    try {
      const [list, wa] = await Promise.all([api.listSources(), api.listWhatsappFiles()]);
      setSources(list);
      setActive(new Set(list.filter((s) => s.active).map((s) => s.id)));
      setFiles(wa);
    } catch (e) {
      onError(e);
    }
  }, [onError]);

  // `claude mcp list` checks every connector, so it can take a while: run it in the background.
  const refreshConnectors = useCallback(async () => {
    setConnectors(null);
    setConnectorsError(null);
    try {
      setConnectors(await api.listConnectors());
    } catch (e) {
      setConnectors([]);
      setConnectorsError(String(e));
    }
  }, []);

  useEffect(() => {
    refreshSources();
    refreshConnectors();
  }, [refreshSources, refreshConnectors]);

  function toggle(id: SourceId) {
    setActive((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      api.setActiveSources([...next]).catch(onError);
      return next;
    });
  }

  const states: SourceState[] = sources.map((s) => {
    if (s.id === "whatsapp") {
      return files.length
        ? { ...s, health: "ok", statusText: `${files.length} chat export${files.length > 1 ? "s" : ""} attached` }
        : { ...s, health: "missing", statusText: "No chat export attached — add one in Settings" };
    }
    if (s.id === "saved") {
      const n = s.files ?? 0;
      return n
        ? { ...s, health: "ok", statusText: `${n} supplier answer${n > 1 ? "s" : ""} saved` }
        : { ...s, health: "empty", statusText: "Empty: supplier answers you record are saved here" };
    }
    if (connectors === null) return { ...s, health: "checking", statusText: "Checking connection…" };
    const c = connectors.find((c) => c.name === s.connector);
    if (!c) return { ...s, health: "missing", statusText: `Connector "${s.connector}" not found in Claude` };
    return c.connected
      ? { ...s, health: "ok", statusText: `Connected via ${c.name}` }
      : { ...s, health: "warn", statusText: `${c.status} — reconnect it in claude.ai connector settings` };
  });

  return {
    sources: states,
    active,
    toggle,
    connectors,
    connectorsError,
    files,
    refreshSources,
    refreshConnectors,
  };
}
