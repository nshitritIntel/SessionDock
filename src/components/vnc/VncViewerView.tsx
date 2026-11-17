import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { ExternalLink, Eye, EyeOff, Maximize2, MonitorUp, PlugZap, Unplug } from "lucide-react";
import { getCredentialSecret, launchVncViewer, startVncProxy, stopVncProxy } from "../../api/commands";
import { type TerminalTab, useAppStore } from "../../stores/appStore";
import type { Session } from "../../types";

export function VncViewerView({ tab, session, active }: { tab: TerminalTab; session?: Session; active: boolean }) {
  const targetRef = useRef<HTMLDivElement>(null);
  const rfbRef = useRef<RFB>();
  const proxyIdRef = useRef<string>();
  const connectingRef = useRef(false);
  const [error, setError] = useState<string>();
  const [credentialsRequired, setCredentialsRequired] = useState(false);
  const [username, setUsername] = useState(tab.username ?? session?.username ?? "");
  const [password, setPassword] = useState("");
  const [viewOnly, setViewOnly] = useState(false);
  const [scaled, setScaled] = useState(true);
  const updateTabStatus = useAppStore((state) => state.updateTabStatus);
  const port = tab.port || session?.port || 5900;

  const disconnect = async () => {
    rfbRef.current?.disconnect();
    rfbRef.current = undefined;
    if (proxyIdRef.current) {
      const proxyId = proxyIdRef.current;
      proxyIdRef.current = undefined;
      await stopVncProxy(proxyId).catch(() => undefined);
    }
    connectingRef.current = false;
    setPassword("");
    setCredentialsRequired(false);
    updateTabStatus(tab.id, "disconnected");
  };

  const connect = async () => {
    if (!targetRef.current || rfbRef.current || connectingRef.current) return;
    connectingRef.current = true;
    setError(undefined);
    setCredentialsRequired(false);
    updateTabStatus(tab.id, "connecting");
    try {
      const proxy = await startVncProxy(tab.host, port);
      proxyIdRef.current = proxy.id;
      let secret = tab.password ?? "";
      const credentialProfileId = tab.credentialProfileId ?? session?.credential_profile_id;
      if (!secret && credentialProfileId) secret = (await getCredentialSecret(credentialProfileId)) ?? "";
      if (tab.password) {
        useAppStore.setState((state) => ({
          tabs: state.tabs.map((current) => current.id === tab.id ? { ...current, password: undefined } : current),
        }));
      }
      const rfb = new RFB(targetRef.current, proxy.url, {
        shared: true,
        credentials: secret ? { username, password: secret } : undefined,
      });
      secret = "";
      rfb.scaleViewport = scaled;
      rfb.resizeSession = true;
      rfb.showDotCursor = true;
      rfb.qualityLevel = 7;
      rfb.compressionLevel = 2;
      rfb.viewOnly = viewOnly;
      rfb.addEventListener("connect", () => {
        connectingRef.current = false;
        setCredentialsRequired(false);
        updateTabStatus(tab.id, "connected");
      });
      rfb.addEventListener("credentialsrequired", () => {
        connectingRef.current = false;
        setCredentialsRequired(true);
      });
      rfb.addEventListener("securityfailure", (event) => {
        connectingRef.current = false;
        setError(event.detail.reason || "VNC authentication failed");
        updateTabStatus(tab.id, "auth_failed");
        rfb.disconnect();
      });
      rfb.addEventListener("disconnect", (event) => {
        connectingRef.current = false;
        rfbRef.current = undefined;
        setPassword("");
        setCredentialsRequired(false);
        if (proxyIdRef.current) {
          const proxyId = proxyIdRef.current;
          proxyIdRef.current = undefined;
          void stopVncProxy(proxyId).catch(() => undefined);
        }
        if (!event.detail.clean) {
          setError("The VNC connection ended unexpectedly.");
          updateTabStatus(tab.id, "error");
        } else updateTabStatus(tab.id, "disconnected");
      });
      rfbRef.current = rfb;
    } catch (cause) {
      connectingRef.current = false;
      setPassword("");
      setError(String(cause));
      updateTabStatus(tab.id, "error");
      if (proxyIdRef.current) {
        const proxyId = proxyIdRef.current;
        proxyIdRef.current = undefined;
        await stopVncProxy(proxyId).catch(() => undefined);
      }
    }
  };

  useEffect(() => {
    if (active && !rfbRef.current) void connect();
  }, [active]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => () => {
    rfbRef.current?.disconnect();
    if (proxyIdRef.current) void stopVncProxy(proxyIdRef.current).catch(() => undefined);
  }, []);

  const submitCredentials = (event: React.FormEvent) => {
    event.preventDefault();
    if (!password || !rfbRef.current) return;
    rfbRef.current.sendCredentials({ username, password });
    setPassword("");
    setCredentialsRequired(false);
    connectingRef.current = true;
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-black">
      <div className="flex h-10 flex-shrink-0 items-center gap-2 border-b border-dock-border bg-dock-surface px-2">
        <MonitorUp size={14} className="text-sky-400" />
        <span className="min-w-0 flex-1 truncate text-[11px] text-dock-text-muted">{tab.host}:{port}</span>
        <button type="button" className="icon-button" title={viewOnly ? "Enable remote input" : "View only"} onClick={() => { const next = !viewOnly; setViewOnly(next); if (rfbRef.current) rfbRef.current.viewOnly = next; }}>{viewOnly ? <Eye size={13} /> : <EyeOff size={13} />}</button>
        <button type="button" className="icon-button" title={scaled ? "Show at native size" : "Fit to workspace"} onClick={() => { const next = !scaled; setScaled(next); if (rfbRef.current) rfbRef.current.scaleViewport = next; }}><Maximize2 size={13} /></button>
        {rfbRef.current ? <button type="button" className="icon-button" title="Disconnect" onClick={() => void disconnect()}><Unplug size={13} /></button> : <button type="button" className="icon-button" title="Connect" onClick={() => void connect()}><PlugZap size={13} /></button>}
        <button type="button" className="icon-button" title="Open in external viewer" onClick={() => void launchVncViewer(tab.host, port)}><ExternalLink size={13} /></button>
      </div>

      <div className="relative min-h-0 flex-1 overflow-hidden">
        <div ref={targetRef} className="h-full w-full overflow-hidden [&>div]:h-full [&_canvas]:max-h-full [&_canvas]:max-w-full" />
        {credentialsRequired && (
          <form onSubmit={submitCredentials} className="absolute inset-0 flex items-center justify-center bg-black/80 p-5">
            <div className="w-full max-w-sm rounded-md border border-dock-border bg-dock-surface p-4">
              <h2 className="mb-3 text-sm font-semibold text-dock-text">VNC credentials</h2>
              <label className="mb-2 block text-[11px] text-dock-text-muted">Username<input autoComplete="username" value={username} onChange={(event) => setUsername(event.target.value)} className="form-input mt-1 w-full" /></label>
              <label className="block text-[11px] text-dock-text-muted">Password<input autoFocus required autoComplete="current-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} className="form-input mt-1 w-full" /></label>
              <button className="mt-3 w-full rounded-md bg-dock-accent px-3 py-2 text-xs font-medium text-white">Connect</button>
            </div>
          </form>
        )}
        {error && <div className="absolute inset-x-4 bottom-4 flex items-center gap-3 rounded-md border border-red-500/30 bg-dock-surface p-3 text-xs text-dock-error"><span className="flex-1">{error}</span><button className="rounded bg-dock-accent px-2.5 py-1.5 text-white" onClick={() => void connect()}>Reconnect</button></div>}
      </div>
    </div>
  );
}