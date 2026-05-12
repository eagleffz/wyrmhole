import {
  listen,
  sendFile,
  cancelSend,
  cancelDownload,
  cancelAllTransfers,
  requestFile,
  cancelConnection,
  acceptFile,
  denyFile,
  getReceivedFiles,
  getSentFiles,
} from "./api";
import { useEffect, useState, useRef } from "react";
import { Toaster, toast } from "sonner";
import ReceiveFileCard from "./RecieveFileCardComponent";
import SentFileCard from "./SentFileCard";
import ActiveDownloadCard from "./ActiveDownloadCard";
import ActiveSendCard from "./ActiveSendCard";
import PendingFileOfferCard from "./PendingFileOfferCard";
import ConnectingCard from "./ConnectingCard";
import SettingsMenu from "./SettingsMenu";
import { FileIcon } from "./FileIcon";
import "./App.css";

interface ReceivedFile {
  connection_type: string;
  download_time: string;
  download_url: string;
  file_extension: string;
  file_name: string;
  file_size: number;
  peer_address: string;
}

interface SentFile {
  file_name: string;
  file_size: number;
  file_extension: string;
  file_paths?: string[];
  file_path?: string;
  send_time: string;
  connection_code: string;
}

interface DownloadProgress {
  id: string;
  file_name: string;
  transferred: number;
  total: number;
  percentage: number;
  error?: string;
}

interface SendProgress {
  id: string;
  file_name: string;
  sent: number;
  total: number;
  percentage: number;
  error?: string;
  code?: string;
  status?: string;
}

interface PendingFileOffer {
  id: string;
  file_name: string;
  file_size?: number;
}

function App() {
  const [receiveCode, setReceiveCode] = useState("");
  const [selectedFiles, setSelectedFiles] = useState<File[] | null>(null);
  const [folderName, setFolderName] = useState<string>("");
  const [receivedFiles, setReceivedFiles] = useState<ReceivedFile[]>([]);
  const [sentFiles, setSentFiles] = useState<SentFile[]>([]);
  const [historyTab, setHistoryTab] = useState<"received" | "sent">("received");
  const [historySearch, setHistorySearch] = useState("");
  const [historyMinSizeMb, setHistoryMinSizeMb] = useState("");
  const [historySizeMode, setHistorySizeMode] = useState<"atLeast" | "atMost">("atLeast");
  const [historyDateFrom, setHistoryDateFrom] = useState("");
  const [historyDateMode, setHistoryDateMode] = useState<"after" | "before">("after");
  const [dateButtonAnimating, setDateButtonAnimating] = useState(false);
  const [sizeButtonAnimating, setSizeButtonAnimating] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<Map<string, DownloadProgress>>(new Map());
  const [sendProgress, setSendProgress] = useState<Map<string, SendProgress>>(new Map());
  const [pendingFileOffers, setPendingFileOffers] = useState<Map<string, PendingFileOffer>>(new Map());
  const [defaultFolderNameFormat, setDefaultFolderNameFormat] = useState<string>("#-files-via-wyrmhole");
  const [connectingCodes, setConnectingCodes] = useState<Map<string, string>>(new Map());
  const [isDragging, setIsDragging] = useState(false);
  const cancelledConnections = useRef<Set<string>>(new Set());
  const connectionCodeToasts = useRef<Map<string | number, string>>(new Map());
  const fileInputRef = useRef<HTMLInputElement>(null);
  const appendFileInputRef = useRef<HTMLInputElement>(null);

  async function deny_file_receive(id: string) {
    try {
      await denyFile(id);
      setPendingFileOffers((prev) => {
        const next = new Map(prev);
        next.delete(id);
        return next;
      });
    } catch (error) {
      console.error("Error denying file:", error);
    }
  }

  async function accept_file_receive(id: string, file_name?: string) {
    try {
      setPendingFileOffers((prev) => {
        const next = new Map(prev);
        next.delete(id);
        return next;
      });
      setDownloadProgress((prev) => {
        const next = new Map(prev);
        next.set(id, { id, file_name: file_name || "Unknown file", transferred: 0, total: 0, percentage: 0 });
        return next;
      });
      await acceptFile(id);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setDownloadProgress((prev) => {
        const next = new Map(prev);
        const existing = next.get(id);
        if (existing) {
          next.set(id, { ...existing, error: errorMessage });
        } else {
          next.set(id, { id, file_name: file_name || "Unknown file", transferred: 0, total: 0, percentage: 0, error: errorMessage });
        }
        return next;
      });
    }
  }

  function select_files() {
    fileInputRef.current?.click();
  }

  function append_files() {
    appendFileInputRef.current?.click();
  }

  function handle_file_input(e: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(e.target.files ?? []);
    if (files.length > 0) {
      setSelectedFiles(files);
      setFolderName("");
    }
    e.target.value = "";
  }

  function handle_append_input(e: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(e.target.files ?? []);
    if (files.length === 0) return;
    setSelectedFiles((prev) => {
      const existing = prev ?? [];
      const merged = [...existing];
      for (const f of files) {
        if (!merged.find((ef) => ef.name === f.name && ef.size === f.size)) {
          merged.push(f);
        }
      }
      return merged.length > 0 ? merged : null;
    });
    e.target.value = "";
  }

  async function send_files() {
    if (!selectedFiles || selectedFiles.length === 0) return;
    const sendId = crypto.randomUUID();
    const displayName = selectedFiles.length === 1 ? selectedFiles[0].name : "Preparing...";

    setSendProgress((prev) => {
      const next = new Map(prev);
      next.set(sendId, { id: sendId, file_name: displayName, sent: 0, total: 0, percentage: 0, status: "preparing" });
      return next;
    });

    try {
      await sendFile(selectedFiles, sendId, folderName.trim() || undefined);
      setFolderName("");
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setSendProgress((prev) => {
        const next = new Map(prev);
        const existing = next.get(sendId);
        if (existing) {
          next.set(sendId, { ...existing, error: errorMessage });
        } else {
          next.set(sendId, { id: sendId, file_name: displayName, sent: 0, total: 0, percentage: 0, error: errorMessage });
        }
        return next;
      });
    }
  }

  async function request_file() {
    if (!receiveCode.trim()) return;
    const codeToUse = receiveCode.trim();
    const connectionId = `conn-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;

    setConnectingCodes((prev) => {
      const next = new Map(prev);
      next.set(connectionId, codeToUse);
      return next;
    });
    setReceiveCode("");

    try {
      const data = await requestFile(codeToUse, connectionId);
      const wasCancelled = cancelledConnections.current.has(connectionId);
      cancelledConnections.current.delete(connectionId);
      setConnectingCodes((prev) => {
        const next = new Map(prev);
        next.delete(connectionId);
        return next;
      });

      if (!data || !data.id || !data.file_name) {
        if (!wasCancelled) toast.error("Invalid file offer from backend.");
        return;
      }
      if (wasCancelled) {
        try { await denyFile(data.id); } catch {}
        return;
      }
      setPendingFileOffers((prev) => {
        const next = new Map(prev);
        next.set(data.id, { id: data.id, file_name: data.file_name, file_size: data.file_size });
        return next;
      });
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e);
      const wasCancelled = cancelledConnections.current.has(connectionId);
      cancelledConnections.current.delete(connectionId);
      setConnectingCodes((prev) => {
        const next = new Map(prev);
        next.delete(connectionId);
        return next;
      });
      if (!wasCancelled) {
        toast.error(errorMessage || "Failed to connect to sender.");
      }
    }
  }

  async function cancelConnectionById(code: string) {
    let connectionId: string | null = null;
    setConnectingCodes((prev) => {
      const next = new Map(prev);
      for (const [id, connCode] of next.entries()) {
        if (connCode === code) {
          connectionId = id;
          cancelledConnections.current.add(id);
          next.delete(id);
          break;
        }
      }
      return next;
    });
    if (connectionId) {
      try {
        await cancelConnection(connectionId);
        toast.success("Connection cancelled");
      } catch {}
    }
  }

  async function refresh_received_files() {
    try {
      const response = await getReceivedFiles();
      if (Array.isArray(response)) setReceivedFiles(response as ReceivedFile[]);
    } catch {}
  }

  async function refresh_sent_files() {
    try {
      const response = await getSentFiles();
      if (Array.isArray(response)) setSentFiles(response as SentFile[]);
    } catch {}
  }

  function remove_file_at_index(idx: number) {
    setSelectedFiles((prev) => {
      if (!prev) return null;
      const next = prev.filter((_, i) => i !== idx);
      return next.length > 0 ? next : null;
    });
  }

  async function cancel_all_transfers() {
    try {
      await cancelAllTransfers();
      setSendProgress(new Map());
      setDownloadProgress(new Map());
      setPendingFileOffers(new Map());
      setConnectingCodes(new Map());
      cancelledConnections.current = new Set();
      toast.success("All active transfers cancelled", { duration: 3000 });
    } catch {
      toast.error("Failed to cancel all transfers");
    }
  }

  useEffect(() => {
    refresh_received_files();
    refresh_sent_files();
  }, []);

  useEffect(() => {
    const unlisten = listen("received-file-added", () => refresh_received_files());
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("sent-file-added", () => refresh_sent_files());
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("default-folder-name-format-updated", (payload) => {
      const p = payload as { value: string };
      setDefaultFolderNameFormat(p.value);
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("connection-code", (payload) => {
      const p = payload as { status: string; code?: string; message?: string; send_id?: string };
      if (p.status === "success" && p.send_id) {
        setSendProgress((prev) => {
          const next = new Map(prev);
          const existing = next.get(p.send_id!);
          if (existing) next.set(p.send_id!, { ...existing, code: p.code });
          return next;
        });
        const codeToCopy = p.code ?? "";
        const toastId = toast(`Connection code: ${codeToCopy}`, {
          duration: 10000,
          className: "connection-code-toast",
          description: "Click anywhere to copy",
          style: { gap: "2px" },
        });
        connectionCodeToasts.current.set(toastId, codeToCopy);
      } else if (p.status === "success") {
        const codeToCopy = p.code ?? "";
        const toastId = toast(`Connection code: ${codeToCopy}`, {
          duration: 999999999,
          className: "connection-code-toast",
          description: "Click anywhere to copy",
          style: { gap: "2px" },
        });
        connectionCodeToasts.current.set(toastId, codeToCopy);
      } else {
        toast.error(p.message ?? "Unknown error in mailbox creation");
      }
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("download-progress", (payload) => {
      const p = payload as DownloadProgress;
      setDownloadProgress((prev) => {
        const next = new Map(prev);
        next.set(p.id, p);
        return next;
      });
      if (p.percentage >= 100) {
        setTimeout(() => {
          toast.success(`Downloaded ${p.file_name}`, { duration: 5000 });
          setDownloadProgress((prev) => {
            const next = new Map(prev);
            next.delete(p.id);
            return next;
          });
        }, 500);
      }
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("download-error", (payload) => {
      const p = payload as { id: string; file_name: string; error: string };
      if (p.error === "Transfer cancelled by user") {
        setDownloadProgress((prev) => {
          const next = new Map(prev);
          next.delete(p.id);
          return next;
        });
        return;
      }
      setDownloadProgress((prev) => {
        const next = new Map(prev);
        const existing = next.get(p.id);
        if (existing) {
          next.set(p.id, { ...existing, error: p.error });
        } else {
          next.set(p.id, { id: p.id, file_name: p.file_name, transferred: 0, total: 0, percentage: 0, error: p.error });
        }
        return next;
      });
      toast.error(`Download failed: ${p.file_name}`, { duration: 5000 });
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("send-progress", (payload) => {
      const p = payload as SendProgress;
      setSendProgress((prev) => {
        const next = new Map(prev);
        next.set(p.id, p);
        return next;
      });
      if (p.percentage >= 100) {
        setTimeout(() => {
          toast.success(`Sent ${p.file_name}`, { duration: 5000 });
          setSendProgress((prev) => {
            const next = new Map(prev);
            next.delete(p.id);
            return next;
          });
        }, 500);
      }
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const unlisten = listen("send-error", (payload) => {
      const p = payload as { id: string; file_name: string; error: string };
      setSendProgress((prev) => {
        const next = new Map(prev);
        const existing = next.get(p.id);
        if (existing) {
          next.set(p.id, { ...existing, error: p.error });
        } else {
          next.set(p.id, { id: p.id, file_name: p.file_name, sent: 0, total: 0, percentage: 0, error: p.error });
        }
        return next;
      });
      if (p.error !== "Transfer cancelled by user") {
        toast.error(`Send failed: ${p.file_name}`, { duration: 5000 });
      }
    });
    return unlisten;
  }, []);

  useEffect(() => {
    const handleToastClick = async (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      const toastElement = target.closest("[data-sonner-toast].connection-code-toast") as HTMLElement;
      if (toastElement) {
        const titleElement = toastElement.querySelector("[data-title]");
        const toastTitleText = titleElement?.textContent || "";
        const codeMatch = toastTitleText.match(/Connection code: (.+)/);
        if (codeMatch && codeMatch[1]) {
          const codeToCopy = codeMatch[1].trim();
          await navigator.clipboard.writeText(codeToCopy);
          toast.success("Code copied to clipboard");
          for (const [toastId, storedCode] of connectionCodeToasts.current.entries()) {
            if (storedCode === codeToCopy) {
              toast.dismiss(toastId);
              connectionCodeToasts.current.delete(toastId);
              break;
            }
          }
        }
      }
    };
    document.addEventListener("click", handleToastClick);
    return () => document.removeEventListener("click", handleToastClick);
  }, []);

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node)) {
      setIsDragging(false);
    }
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      setSelectedFiles((prev) => {
        if (!prev) return files;
        const merged = [...prev];
        for (const f of files) {
          if (!merged.find((ef) => ef.name === f.name && ef.size === f.size)) {
            merged.push(f);
          }
        }
        return merged;
      });
    }
  };

  return (
    <div
      className="app-container h-screen glass-background flex flex-col overflow-hidden"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <Toaster position="bottom-right" />

      <input ref={fileInputRef} type="file" multiple className="hidden" onChange={handle_file_input} />
      <input ref={appendFileInputRef} type="file" multiple className="hidden" onChange={handle_append_input} />

      <div
        className="pointer-events-none fixed inset-0 z-50 rounded-lg transition-all duration-200"
        style={{
          boxShadow: isDragging
            ? "inset 0 0 0 4px rgba(59, 130, 246, 0.7), inset 0 0 40px 0 rgba(59, 130, 246, 0.15), inset 0 0 80px 0 rgba(59, 130, 246, 0.06)"
            : "none",
          opacity: isDragging ? 1 : 0,
        }}
      />

      <nav className="glass-navbar flex-shrink-0 z-10">
        <div className="px-3 sm:px-6 py-2 sm:py-2.5 flex justify-between items-center">
          <h1 className="text-lg sm:text-2xl xl:text-3xl font-bold flex items-center select-none gap-1 sm:gap-2 text-gray-800">
            <span className="spin-on-hover cursor-pointer text-lg sm:text-2xl xl:text-3xl flex items-center">🌀</span>
            <span className="gradient-shimmer flex items-center">wyrmhole</span>
          </h1>
          <SettingsMenu onFolderNameFormatChange={setDefaultFolderNameFormat} />
        </div>
      </nav>

      <div className="flex-1 overflow-y-auto min-h-0" style={{ scrollbarWidth: "thin" }}>
        <div className="max-w-6xl mx-auto px-3 sm:px-6 py-3 sm:py-4 select-none">
          <div className={`mb-3 sm:mb-4 ${sendProgress.size === 0 && downloadProgress.size === 0 ? "hidden md:block" : ""}`}>
            <div className="flex items-center justify-between mb-2">
              <h2 className="text-xs sm:text-sm xl:text-base font-semibold text-gray-800 select-none cursor-default">Active Transfers</h2>
              {(sendProgress.size > 0 || downloadProgress.size > 0) && (
                <button type="button" onClick={cancel_all_transfers} className="text-[10px] sm:text-xs xl:text-sm text-red-600 hover:text-red-700 px-2 py-1 rounded-xl border border-red-200 hover:border-red-300 bg-red-50/70 cursor-pointer transition-colors">
                  Cancel all
                </button>
              )}
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-2 sm:gap-3">
              <div className={`rounded-2xl overflow-hidden md:h-[140px] flex flex-col ${sendProgress.size === 0 ? "hidden md:flex" : ""}`} style={{ background: "rgba(255,255,255,0.5)", backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)", border: "1px solid rgba(255,255,255,0.4)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                <div className="px-2 sm:px-3 py-1.5 border-b border-white/20 flex-shrink-0" style={{ background: "rgba(59,130,246,0.15)", backdropFilter: "blur(8px)", WebkitBackdropFilter: "blur(8px)" }}>
                  <p className="text-[9px] sm:text-[10px] xl:text-xs font-semibold text-blue-700 uppercase tracking-wide">Sending {sendProgress.size > 0 && `(${sendProgress.size})`}</p>
                </div>
                <div className={`overflow-y-auto md:flex-1 ${sendProgress.size === 0 ? "flex items-center justify-center" : ""}`} style={{ scrollbarWidth: "thin" }}>
                  {sendProgress.size > 0 ? (
                    Array.from(sendProgress.values()).map((progress) => (
                      <ActiveSendCard key={progress.id} {...progress} onDismiss={(id) => { setSendProgress((prev) => { const next = new Map(prev); next.delete(id); return next; }); }} onCancel={async (id) => { await cancelSend(id); }} />
                    ))
                  ) : (
                    <div className="text-center text-xs xl:text-sm text-gray-400">No active sends</div>
                  )}
                </div>
              </div>

              <div className={`rounded-2xl overflow-hidden md:h-[140px] flex flex-col ${downloadProgress.size === 0 ? "hidden md:flex" : ""}`} style={{ background: "rgba(255,255,255,0.5)", backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)", border: "1px solid rgba(255,255,255,0.4)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                <div className="px-2 sm:px-3 py-1.5 border-b border-white/20 flex-shrink-0" style={{ background: "rgba(34,197,94,0.15)", backdropFilter: "blur(8px)", WebkitBackdropFilter: "blur(8px)" }}>
                  <p className="text-[9px] sm:text-[10px] xl:text-xs font-semibold text-green-700 uppercase tracking-wide">Receiving {downloadProgress.size > 0 && `(${downloadProgress.size})`}</p>
                </div>
                <div className={`overflow-y-auto md:flex-1 ${downloadProgress.size === 0 ? "flex items-center justify-center" : ""}`} style={{ scrollbarWidth: "thin" }}>
                  {downloadProgress.size > 0 ? (
                    Array.from(downloadProgress.values()).map((progress) => (
                      <ActiveDownloadCard key={progress.id} {...progress} onDismiss={(id) => { setDownloadProgress((prev) => { const next = new Map(prev); next.delete(id); return next; }); }} onCancel={async (id) => { await cancelDownload(id); }} />
                    ))
                  ) : (
                    <div className="text-center text-xs xl:text-sm text-gray-400">No active downloads</div>
                  )}
                </div>
              </div>
            </div>
          </div>

          <div className="mb-3 sm:mb-4">
            <h2 className="text-xs sm:text-sm xl:text-base font-semibold text-gray-800 mb-2 select-none cursor-default">Actions</h2>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-2 sm:gap-3">
              <div className="rounded-2xl overflow-hidden min-h-[200px] md:max-h-[240px] flex flex-col" style={{ background: "rgba(255,255,255,0.5)", backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)", border: "1px solid rgba(255,255,255,0.4)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                <div className="px-3 py-2 border-b border-white/20 flex-shrink-0">
                  <div className="flex items-center justify-between gap-2">
                    <h3 className="text-xs sm:text-sm xl:text-base font-semibold text-gray-700 flex-shrink-0">Send Files</h3>
                    <div className="flex items-center gap-2 flex-1 justify-end">
                      {selectedFiles && selectedFiles.length > 1 && (
                        <input type="text" value={folderName} onChange={(e) => setFolderName(e.target.value)}
                          placeholder={`Folder Name: ${(defaultFolderNameFormat.trim() || "#-files-via-wyrmhole").replace("#", selectedFiles.length.toString())}`}
                          className="flex-1 px-2 py-1 text-xs xl:text-sm rounded-xl focus:outline-none focus:ring-2 focus:ring-blue-500/20 transition-all border border-gray-300/60"
                          style={{ background: "rgba(255,255,255,0.7)", backdropFilter: "blur(16px)", WebkitBackdropFilter: "blur(16px)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }} />
                      )}
                      {selectedFiles && (
                        <button onClick={send_files} className="font-medium flex items-center justify-center gap-1 px-2 py-1 text-white text-xs xl:text-sm rounded-2xl transition-all cursor-pointer flex-shrink-0"
                          style={{ background: "rgba(59,130,246,0.9)", backdropFilter: "blur(4px)", WebkitBackdropFilter: "blur(4px)", border: "1px solid rgba(255,255,255,0.3)", boxShadow: "0 2px 8px 0 rgba(59,130,246,0.4),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="2" stroke="currentColor" className="w-3.5 h-3.5">
                            <path strokeLinecap="round" strokeLinejoin="round" d="M6 12 3.269 3.125A59.769 59.769 0 0 1 21.485 12 59.768 59.768 0 0 1 3.27 20.875L5.999 12Zm0 0h7.5" />
                          </svg>
                          <span>Send</span>
                        </button>
                      )}
                    </div>
                  </div>
                </div>
                <div className="flex-1 flex flex-col p-3 min-h-0">
                  {!selectedFiles ? (
                    <label className={`flex-1 flex flex-col items-center justify-center cursor-pointer border-2 border-dashed transition-all duration-200 rounded-2xl ${isDragging ? "scale-[1.02]" : ""}`}
                      style={{ borderColor: isDragging ? "rgba(59,130,246,0.7)" : "rgba(255,255,255,0.3)", background: isDragging ? "rgba(219,234,254,0.4)" : "rgba(255,255,255,0.2)", backdropFilter: "blur(8px)", WebkitBackdropFilter: "blur(8px)" }}
                      onClick={select_files}>
                      <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className={`w-5 h-5 mb-1 transition-colors ${isDragging ? "text-blue-500" : "text-gray-400"}`}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M7.5 7.5h-.75A2.25 2.25 0 0 0 4.5 9.75v7.5a2.25 2.25 0 0 0 2.25 2.25h7.5a2.25 2.25 0 0 0 2.25-2.25v-7.5a2.25 2.25 0 0 0-2.25-2.25h-.75m0-3-3-3m0 0-3 3m3-3v11.25m6-2.25h.75a2.25 2.25 0 0 1 2.25 2.25v7.5a2.25 2.25 0 0 1-2.25 2.25h-7.5a2.25 2.25 0 0 1-2.25-2.25v-.75" />
                      </svg>
                      <span className={`text-xs xl:text-sm transition-colors ${isDragging ? "text-blue-600 font-medium" : "text-gray-600"}`}>
                        {isDragging ? "Drop files here" : "Click or drag files here"}
                      </span>
                    </label>
                  ) : (
                    <div className="flex-1 flex flex-col min-h-0">
                      <div className="flex items-center justify-between mb-2 flex-shrink-0">
                        <span className="text-xs xl:text-sm font-medium text-gray-700">{selectedFiles.length} {selectedFiles.length === 1 ? "file" : "files"}</span>
                        <div className="flex items-center gap-2">
                          <button onClick={(e) => { e.stopPropagation(); append_files(); }} className="text-[10px] xl:text-xs text-blue-600 hover:text-blue-700 transition-colors cursor-pointer" title="Add more files">Add files</button>
                          <button onClick={(e) => { e.stopPropagation(); setSelectedFiles(null); setFolderName(""); }} className="text-[10px] xl:text-xs text-gray-500 hover:text-red-600 transition-colors cursor-pointer" title="Clear">Clear</button>
                        </div>
                      </div>
                      <div className="flex-1 overflow-y-auto pr-1 min-h-0" style={{ scrollbarWidth: "thin" }}>
                        {selectedFiles.length === 1 ? (
                          <div className="group flex items-center gap-2 p-2 rounded-xl" style={{ background: "rgba(255,255,255,0.3)", backdropFilter: "blur(16px)", WebkitBackdropFilter: "blur(16px)", border: "1px solid rgba(255,255,255,0.3)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                            <FileIcon fileName={selectedFiles[0].name} className="w-4 h-4 flex-shrink-0" />
                            <p className="text-xs xl:text-sm font-medium text-gray-900 truncate flex-1">{selectedFiles[0].name}</p>
                            <button onClick={(e) => { e.stopPropagation(); remove_file_at_index(0); }} className="p-0.5 rounded hover:bg-red-100 transition-colors cursor-pointer opacity-0 group-hover:opacity-100" title="Remove">
                              <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" className="w-4 h-4 fill-gray-400 hover:fill-red-600"><path d="M4.646 4.646a.5.5 0 0 1 .708 0L8 7.293l2.646-2.647a.5.5 0 0 1 .708.708L8.707 8l2.647 2.646a.5.5 0 0 1-.708.708L8 8.707l-2.646 2.647a.5.5 0 0 1-.708-.708L7.293 8 4.646 5.354a.5.5 0 0 1 0-.708" /></svg>
                            </button>
                          </div>
                        ) : (
                          <div className="space-y-1">
                            {selectedFiles.map((file, idx) => (
                              <div key={idx} className="group flex items-center gap-2 p-1.5 rounded-xl" style={{ background: "rgba(255,255,255,0.3)", backdropFilter: "blur(16px)", WebkitBackdropFilter: "blur(16px)", border: "1px solid rgba(255,255,255,0.3)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                                <FileIcon fileName={file.name} className="w-3.5 h-3.5 flex-shrink-0" />
                                <p className="text-[11px] xl:text-xs font-medium text-gray-900 truncate flex-1">{file.name}</p>
                                <button onClick={(e) => { e.stopPropagation(); remove_file_at_index(idx); }} className="p-0.5 rounded hover:bg-red-100 transition-colors cursor-pointer opacity-0 group-hover:opacity-100" title="Remove">
                                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" className="w-4 h-4 fill-gray-400 hover:fill-red-600"><path d="M4.646 4.646a.5.5 0 0 1 .708 0L8 7.293l2.646-2.647a.5.5 0 0 1 .708.708L8.707 8l2.647 2.646a.5.5 0 0 1-.708.708L8 8.707l-2.646 2.647a.5.5 0 0 1-.708-.708L7.293 8 4.646 5.354a.5.5 0 0 1 0-.708" /></svg>
                                </button>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div className="rounded-2xl overflow-hidden flex flex-col" style={{ background: "rgba(255,255,255,0.5)", backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)", border: "1px solid rgba(255,255,255,0.4)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                <div className="px-3 py-2 border-b border-white/20 flex-shrink-0">
                  <form onSubmit={(e) => { e.preventDefault(); request_file(); }} className="flex items-center gap-2">
                    <h3 className="text-xs sm:text-sm xl:text-base font-semibold text-gray-700 flex-shrink-0">Receive Files</h3>
                    <input value={receiveCode} onChange={(e) => setReceiveCode(e.target.value)} placeholder="Enter code: ex. 7-helpful-tiger"
                      className="flex-1 px-2 py-1 text-xs xl:text-sm rounded-xl focus:outline-none focus:ring-2 focus:ring-blue-500/20 transition-all"
                      style={{ background: "rgba(255,255,255,0.3)", backdropFilter: "blur(16px)", WebkitBackdropFilter: "blur(16px)", border: "1px solid rgba(255,255,255,0.3)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }} />
                    <button type="submit" className="font-medium flex items-center justify-center gap-1 px-2 py-1 text-white text-xs xl:text-sm rounded-2xl transition-all cursor-pointer flex-shrink-0"
                      style={{ background: "rgba(59,130,246,0.9)", backdropFilter: "blur(4px)", WebkitBackdropFilter: "blur(4px)", border: "1px solid rgba(255,255,255,0.3)", boxShadow: "0 2px 8px 0 rgba(59,130,246,0.4),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
                      <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="2" stroke="currentColor" className="w-3.5 h-3.5">
                        <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 0 0 5.25 21h13.5A2.25 2.25 0 0 0 21 18.75V16.5M16.5 12 12 16.5m0 0L7.5 12m4.5 4.5V3" />
                      </svg>
                      <span>Receive</span>
                    </button>
                  </form>
                </div>
                <div className="flex flex-col p-3">
                  {connectingCodes.size > 0 || pendingFileOffers.size > 0 ? (
                    <div className="flex flex-col">
                      <div className="text-[10px] sm:text-xs xl:text-sm font-semibold text-gray-700 uppercase tracking-wide mb-2 flex-shrink-0">
                        {connectingCodes.size > 0 && pendingFileOffers.size > 0 ? `Connecting (${connectingCodes.size}) • Pending Offers (${pendingFileOffers.size})` : connectingCodes.size > 0 ? `Connecting (${connectingCodes.size})` : `Pending Offers (${pendingFileOffers.size})`}
                      </div>
                      <div className="overflow-y-auto space-y-1 max-h-[200px] md:max-h-[240px]" style={{ scrollbarWidth: "thin" }}>
                        {Array.from(connectingCodes.entries()).map(([id, code]) => (
                          <ConnectingCard key={id} code={code} onCancel={cancelConnectionById} />
                        ))}
                        {Array.from(pendingFileOffers.values()).map((offer) => (
                          <PendingFileOfferCard key={offer.id} {...offer}
                            onAccept={(id) => { const o = pendingFileOffers.get(id); if (o) accept_file_receive(id, o.file_name); }}
                            onDeny={(id) => deny_file_receive(id)} />
                        ))}
                      </div>
                    </div>
                  ) : (
                    <div className="flex items-center justify-center text-xs xl:text-sm text-gray-400 py-2">No pending offers</div>
                  )}
                </div>
              </div>
            </div>
          </div>

          <div>
            <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
              <div className="flex items-center gap-2">
                <h2 className="text-xs sm:text-sm xl:text-base font-semibold text-gray-800 select-none cursor-default">File History</h2>
                <div className="flex items-center gap-1.5 text-xs">
                  <button onClick={() => { if (historyTab !== "received") { setHistoryTab("received"); refresh_received_files(); } }} className={`px-2 py-1 rounded-xl transition-all duration-200 ${historyTab === "received" ? "text-blue-700 font-semibold" : "text-gray-500 hover:text-blue-600"}`}>Received</button>
                  <span className="text-gray-400">/</span>
                  <button onClick={() => { if (historyTab !== "sent") { setHistoryTab("sent"); refresh_sent_files(); } }} className={`px-2 py-1 rounded-xl transition-all duration-200 ${historyTab === "sent" ? "text-blue-700 font-semibold" : "text-gray-500 hover:text-blue-600"}`}>Sent</button>
                </div>
              </div>
              <div className="flex items-center gap-1.5 text-[10px] sm:text-xs">
                <input value={historySearch} onChange={(e) => setHistorySearch(e.target.value)} placeholder="Search filename" className="px-2 py-1 rounded-xl border border-gray-300/60 bg-white/60 focus:outline-none focus:ring-1 focus:ring-blue-400/60" style={{ minWidth: "120px" }} />
                <div className="flex items-center">
                  <button type="button" onClick={() => { setHistoryDateMode((m) => m === "after" ? "before" : "after"); setDateButtonAnimating(true); setTimeout(() => setDateButtonAnimating(false), 200); }} className={`px-2 py-1 border border-gray-300/60 bg-white/60 hover:bg-white/80 hover:cursor-pointer transition-colors rounded-l-xl ${dateButtonAnimating ? "filter-button-click" : ""}`}>
                    {historyDateMode === "after" ? "After" : "Before"}
                  </button>
                  <input type="date" value={historyDateFrom} onChange={(e) => setHistoryDateFrom(e.target.value)} className="px-2 py-1 border border-l-0 border-gray-300/60 bg-white/60 focus:outline-none focus:ring-1 focus:ring-blue-400/60 rounded-r-xl hover:cursor-pointer" />
                </div>
                <div className="flex items-center">
                  <button type="button" onClick={() => { setHistorySizeMode((m) => m === "atLeast" ? "atMost" : "atLeast"); setSizeButtonAnimating(true); setTimeout(() => setSizeButtonAnimating(false), 200); }} className={`px-2 py-1 border border-gray-300/60 bg-white/60 hover:bg-white/80 hover:cursor-pointer transition-colors rounded-l-xl ${sizeButtonAnimating ? "filter-button-click" : ""}`}>
                    {historySizeMode === "atLeast" ? "≥ MB" : "≤ MB"}
                  </button>
                  <input type="number" min="0" value={historyMinSizeMb} onChange={(e) => setHistoryMinSizeMb(e.target.value)} placeholder="MB" className="w-20 px-2 py-1 border border-l-0 border-gray-300/60 bg-white/60 focus:outline-none focus:ring-1 focus:ring-blue-400/60 rounded-r-xl hover:cursor-pointer" />
                </div>
              </div>
            </div>

            <div className="rounded-2xl overflow-hidden" style={{ background: "rgba(255,255,255,0.5)", backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)", border: "1px solid rgba(255,255,255,0.4)", boxShadow: "0 2px 8px 0 rgba(0,0,0,0.05),inset 0 1px 0 0 rgba(255,255,255,0.3)" }}>
              <div className="grid grid-cols-[2fr_1fr_1fr] select-none border-b border-white/20 px-2 sm:px-3 py-1.5 text-[9px] sm:text-[10px] xl:text-xs font-semibold text-gray-600 uppercase tracking-wide flex-shrink-0" style={{ background: "rgba(255,255,255,0.3)", backdropFilter: "blur(8px)", WebkitBackdropFilter: "blur(8px)" }}>
                <div className="truncate">Filename</div>
                <div className="truncate">Extension</div>
                <div className="truncate">Size</div>
              </div>
              <div className="max-h-48 sm:max-h-64 overflow-y-auto" style={{ scrollbarWidth: "thin" }}>
                {historyTab === "received" ? (
                  receivedFiles.length > 0 ? (
                    <div className="divide-y divide-gray-100">
                      {receivedFiles.slice().reverse().filter((file) => {
                        const nameMatch = historySearch ? file.file_name.toLowerCase().includes(historySearch.toLowerCase()) : true;
                        let sizeMatch = true;
                        if (historyMinSizeMb.trim() !== "") {
                          const mb = Number(historyMinSizeMb);
                          if (!Number.isNaN(mb) && mb > 0) {
                            const threshold = mb * 1024 * 1024;
                            sizeMatch = historySizeMode === "atLeast" ? file.file_size >= threshold : file.file_size <= threshold;
                          }
                        }
                        let dateMatch = true;
                        if (historyDateFrom) {
                          const boundary = new Date(historyDateFrom);
                          const when = new Date(file.download_time);
                          if (!Number.isNaN(boundary.getTime())) {
                            dateMatch = historyDateMode === "after" ? when >= boundary : when <= boundary;
                          }
                        }
                        return nameMatch && sizeMatch && dateMatch;
                      }).map((file, idx) => (
                        <ReceiveFileCard key={idx} {...file} />
                      ))}
                    </div>
                  ) : (
                    <div className="flex items-center justify-center h-48 sm:h-64 text-xs sm:text-sm text-gray-400">No Received File History</div>
                  )
                ) : sentFiles.length > 0 ? (
                  <div className="divide-y divide-gray-100">
                    {sentFiles.slice().reverse().filter((file) => {
                      const nameMatch = historySearch ? file.file_name.toLowerCase().includes(historySearch.toLowerCase()) : true;
                      let sizeMatch = true;
                      if (historyMinSizeMb.trim() !== "") {
                        const mb = Number(historyMinSizeMb);
                        if (!Number.isNaN(mb) && mb > 0) {
                          const threshold = mb * 1024 * 1024;
                          sizeMatch = historySizeMode === "atLeast" ? file.file_size >= threshold : file.file_size <= threshold;
                        }
                      }
                      let dateMatch = true;
                      if (historyDateFrom) {
                        const boundary = new Date(historyDateFrom);
                        const when = new Date(file.send_time);
                        if (!Number.isNaN(boundary.getTime())) {
                          dateMatch = historyDateMode === "after" ? when >= boundary : when <= boundary;
                        }
                      }
                      return nameMatch && sizeMatch && dateMatch;
                    }).map((file, idx) => {
                      const fileWithPaths = { ...file, file_paths: file.file_paths || (file.file_path ? [file.file_path] : []) };
                      return <SentFileCard key={idx} {...fileWithPaths} />;
                    })}
                  </div>
                ) : (
                  <div className="flex items-center justify-center h-48 sm:h-64 text-xs sm:text-sm text-gray-400">No Sent File History</div>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
