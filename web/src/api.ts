type EventListener = (payload: unknown) => void;

const wsListeners = new Map<string, Set<EventListener>>();
let ws: WebSocket | null = null;

function connectWS() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  ws = new WebSocket(`${protocol}//${window.location.host}/ws`);

  ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data as string) as { type: string; payload: unknown };
      const listeners = wsListeners.get(msg.type);
      if (listeners) {
        listeners.forEach((fn) => fn(msg.payload));
      }
    } catch {
      // ignore malformed messages
    }
  };

  ws.onclose = () => {
    ws = null;
    setTimeout(connectWS, 2000);
  };

  ws.onerror = () => {
    ws?.close();
  };
}

connectWS();

export function listen(event: string, callback: EventListener): () => void {
  if (!wsListeners.has(event)) wsListeners.set(event, new Set());
  wsListeners.get(event)!.add(callback);
  return () => {
    wsListeners.get(event)?.delete(callback);
  };
}

async function api(path: string, options?: RequestInit): Promise<unknown> {
  const resp = await fetch(path, options);
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text || `HTTP ${resp.status}`);
  }
  const contentType = resp.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    return resp.json();
  }
  return resp.text();
}

export async function sendFile(
  files: File[],
  sendId: string,
  folderName?: string,
): Promise<void> {
  const formData = new FormData();
  formData.append("send_id", sendId);
  if (folderName) formData.append("folder_name", folderName);

  if (files.length === 1) {
    formData.append("file", files[0]);
    await api("/api/send", { method: "POST", body: formData });
  } else {
    files.forEach((f) => formData.append("files", f));
    await api("/api/send-multiple", { method: "POST", body: formData });
  }
}

export async function cancelSend(sendId: string): Promise<void> {
  await api(`/api/send/${sendId}`, { method: "DELETE" });
}

export async function cancelDownload(downloadId: string): Promise<void> {
  await api(`/api/download/${downloadId}`, { method: "DELETE" });
}

export async function cancelAllTransfers(): Promise<void> {
  await api("/api/transfers", { method: "DELETE" });
}

export async function requestFile(
  receiveCode: string,
  connectionId: string,
): Promise<{ id: string; file_name: string; file_size: number }> {
  return api("/api/receive", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ receive_code: receiveCode, connection_id: connectionId }),
  }) as Promise<{ id: string; file_name: string; file_size: number }>;
}

export async function cancelConnection(connectionId: string): Promise<void> {
  await api(`/api/connection/${connectionId}`, { method: "DELETE" });
}

export async function acceptFile(id: string): Promise<void> {
  await api(`/api/accept/${id}`, { method: "POST" });
}

export async function denyFile(id: string): Promise<void> {
  await api(`/api/deny/${id}`, { method: "POST" });
}

export async function getReceivedFiles(): Promise<unknown[]> {
  return api("/api/received-files") as Promise<unknown[]>;
}

export async function getSentFiles(): Promise<unknown[]> {
  return api("/api/sent-files") as Promise<unknown[]>;
}

export interface Settings {
  download_directory: string;
  auto_extract_tarballs: boolean;
  default_folder_name_format: string;
  relay_server_url: string | null;
}

export async function getSettings(): Promise<Settings> {
  return api("/api/settings") as Promise<Settings>;
}

export async function patchSettings(patch: Partial<Omit<Settings, "download_directory">>): Promise<void> {
  await api("/api/settings", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(patch),
  });
}

export async function testRelay(): Promise<string> {
  const result = (await api("/api/test-relay", { method: "POST" })) as { message: string };
  return result.message;
}

export function downloadFileUrl(filename: string): string {
  return `/api/download-file/${encodeURIComponent(filename)}`;
}

export function exportReceivedFilesUrl(): string {
  return "/api/export/received-files";
}

export function exportSentFilesUrl(): string {
  return "/api/export/sent-files";
}
