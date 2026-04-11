import { useEffect, useRef, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { terminalInput, terminalOutput, terminalResize } from "../api/client";

interface Props {
  terminalId: string;
  onClose?: () => void;
  onDead?: () => void;
}

function toBase64(str: string): string {
  const bytes = new TextEncoder().encode(str);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

function fromBase64(b64: string): string {
  if (!b64) return "";
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

export function TerminalView({ terminalId, onClose, onDead }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const pollingRef = useRef<number | null>(null);
  const aliveRef = useRef(true);

  const startPolling = useCallback(() => {
    const poll = async () => {
      if (!aliveRef.current) return;
      try {
        const { data, alive } = await terminalOutput(terminalId);
        if (data) {
          const decoded = fromBase64(data);
          if (decoded && termRef.current) {
            termRef.current.write(decoded);
          }
        }
        if (!alive) {
          aliveRef.current = false;
          termRef.current?.write("\r\n\x1b[31m[terminal exited]\x1b[0m\r\n");
          onDead?.();
          return;
        }
      } catch {
        // Ignore transient errors
      }
      pollingRef.current = window.setTimeout(poll, 50);
    };
    poll();
  }, [terminalId, onDead]);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
      theme: {
        background: "#1a1a1a",
        foreground: "#e0e0e0",
        cursor: "#ED1C24",
        selectionBackground: "rgba(237, 28, 36, 0.3)",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    fit.fit();

    termRef.current = term;
    fitRef.current = fit;

    // Send initial resize
    terminalResize(terminalId, term.rows, term.cols).catch(() => {});

    // Send keystrokes to the backend
    term.onData((data) => {
      if (aliveRef.current) {
        terminalInput(terminalId, toBase64(data)).catch(() => {});
      }
    });

    // Handle resize
    const onResize = () => {
      fit.fit();
      if (aliveRef.current) {
        terminalResize(terminalId, term.rows, term.cols).catch(() => {});
      }
    };
    window.addEventListener("resize", onResize);

    // Also resize when terminal dimensions change
    term.onResize(({ cols, rows }) => {
      if (aliveRef.current) {
        terminalResize(terminalId, rows, cols).catch(() => {});
      }
    });

    // Start polling for output
    startPolling();

    return () => {
      aliveRef.current = false;
      if (pollingRef.current) clearTimeout(pollingRef.current);
      window.removeEventListener("resize", onResize);
      term.dispose();
    };
  }, [terminalId, startPolling]);

  return (
    <div className="terminal-container">
      <div className="terminal-header">
        <span className="terminal-title">{terminalId}</span>
        {onClose && (
          <button className="btn-danger-sm" onClick={onClose}>
            Close
          </button>
        )}
      </div>
      <div ref={containerRef} className="terminal-xterm" />
    </div>
  );
}
