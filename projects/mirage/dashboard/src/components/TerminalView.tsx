import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface Props {
  terminalId: string;
  onClose?: () => void;
  onDead?: () => void;
}

/** WebSocket terminal protocol (binary frames):
 *  Client → Server:
 *    byte[0]=0x00 + data       = PTY input
 *    byte[0]=0x01 + u16LE rows + u16LE cols  = resize
 *  Server → Client:
 *    byte[0]=0x00 + data       = PTY output
 *    byte[0]=0x01              = terminal exited
 */

const WS_BASE = `ws://${window.location.host}`;

function buildInputFrame(data: string): ArrayBuffer {
  const encoded = new TextEncoder().encode(data);
  const buf = new Uint8Array(1 + encoded.length);
  buf[0] = 0x00;
  buf.set(encoded, 1);
  return buf.buffer;
}

function buildResizeFrame(rows: number, cols: number): ArrayBuffer {
  const buf = new ArrayBuffer(5);
  const view = new DataView(buf);
  view.setUint8(0, 0x01);
  view.setUint16(1, rows, true); // little-endian
  view.setUint16(3, cols, true);
  return buf;
}

export function TerminalView({ terminalId, onClose, onDead }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

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
    term.focus();

    termRef.current = term;
    fitRef.current = fit;

    // ── WebSocket connection ─────────────────────────────────────────
    const ws = new WebSocket(`${WS_BASE}/terminal/${terminalId}`);
    ws.binaryType = "arraybuffer";
    wsRef.current = ws;

    ws.onopen = () => {
      // Send initial resize
      ws.send(buildResizeFrame(term.rows, term.cols));
    };

    ws.onmessage = (ev) => {
      const data = new Uint8Array(ev.data as ArrayBuffer);
      if (data.length === 0) return;
      const type = data[0];
      if (type === 0x00 && data.length > 1) {
        // PTY output
        const text = new TextDecoder().decode(data.subarray(1));
        term.write(text);
      } else if (type === 0x01) {
        // Terminal exited
        term.write("\r\n\x1b[31m[terminal exited]\x1b[0m\r\n");
        onDead?.();
      }
    };

    ws.onclose = () => {
      term.write("\r\n\x1b[33m[disconnected]\x1b[0m\r\n");
      onDead?.();
    };

    // Send keystrokes over WebSocket
    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(buildInputFrame(data));
      }
    });

    // Handle window resize
    const onResize = () => {
      fit.fit();
    };
    window.addEventListener("resize", onResize);

    // Send resize when terminal dimensions change
    term.onResize(({ cols, rows }) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(buildResizeFrame(rows, cols));
      }
    });

    return () => {
      window.removeEventListener("resize", onResize);
      ws.close();
      term.dispose();
    };
  }, [terminalId, onDead]);

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
      <div
        ref={containerRef}
        className="terminal-xterm"
        onClick={() => termRef.current?.focus()}
      />
    </div>
  );
}
