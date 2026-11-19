import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { AlertTriangle, ExternalLink, Eye, EyeOff, Maximize2, MonitorUp, PlugZap, ShieldCheck, Unplug } from "lucide-react";
import { useTranslation } from "react-i18next";
import { getCredentialSecret, getVncProxyDiagnostics, launchVncViewer, startVncProxy, stopVncProxy } from "../../api/commands";
import { type TerminalTab, useAppStore } from "../../stores/appStore";
import type { Session } from "../../types";
import { classifyVncDisconnect, classifyVncStartFailure, type VncViewerFailure } from "./vncDiagnostics";

interface ServerVerification {
  fingerprint: string;
  type: string;
}

async function fingerprintPublicKey(publicKey: Uint8Array): Promise<string> {
  if (publicKey.byteLength === 0 || publicKey.byteLength > 4096) throw new Error("Invalid VNC server key");
  const digest = await crypto.subtle.digest("SHA-256", publicKey);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join(":");
}

export function VncViewerView({ tab, session, active }: { tab: TerminalTab; session?: Session; active: boolean }) {
  const { t } = useTranslation();
  const targetRef = useRef<HTMLDivElement>(null);
  const rfbRef = useRef<RFB>();
  const proxyIdRef = useRef<string>();
  const connectingRef = useRef(false);
  const failureRef = useRef(false);
  const [failure, setFailure] = useState<VncViewerFailure>();
  const [serverVerification, setServerVerification] = useState<ServerVerification>();
  const [credentialsRequired, setCredentialsRequired] = useState(false);
  const [username, setUsername] = useState(tab.username ?? session?.username ?? "");
  const [password, setPassword] = useState("");
  const [viewOnly, setViewOnly] = useState(false);
  const [scaled, setScaled] = useState(true);
  const updateTabStatus = useAppStore((state) => state.updateTabStatus);
  const host = session?.host ?? tab.host;
  const port = session?.port ?? tab.port ?? 5900;

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
    setServerVerification(undefined);
    updateTabStatus(tab.id, "disconnected");
  };

  const connect = async () => {
    if (!targetRef.current || rfbRef.current || connectingRef.current) return;
    connectingRef.current = true;
    failureRef.current = false;
    setFailure(undefined);
    setServerVerification(undefined);
    setCredentialsRequired(false);
    updateTabStatus(tab.id, "connecting");
    try {
      const proxy = await startVncProxy(host, port);
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
        failureRef.current = true;
        setFailure({ kind: "authentication", technicalDetails: [`Security status: ${event.detail.status}`] });
        updateTabStatus(tab.id, "auth_failed");
        rfb.disconnect();
      });
      rfb.addEventListener("serververification", (event) => {
        void fingerprintPublicKey(event.detail.publickey).then((fingerprint) => {
          setServerVerification({ fingerprint, type: event.detail.type });
          connectingRef.current = false;
        }).catch(() => {
          failureRef.current = true;
          setFailure({ kind: "protocol" });
          updateTabStatus(tab.id, "error");
          rfb.disconnect();
        });
      });
      rfb.addEventListener("disconnect", (event) => {
        connectingRef.current = false;
        rfbRef.current = undefined;
        setPassword("");
        setCredentialsRequired(false);
        setServerVerification(undefined);
        const proxyId = proxyIdRef.current;
        proxyIdRef.current = undefined;
        void (async () => {
          const diagnostics = proxyId
            ? await getVncProxyDiagnostics(proxyId).catch(() => undefined)
            : undefined;
          if (proxyId) await stopVncProxy(proxyId).catch(() => undefined);
          if (!event.detail.clean && !failureRef.current) {
            failureRef.current = true;
            setFailure(classifyVncDisconnect(diagnostics));
            updateTabStatus(tab.id, "error");
          } else if (event.detail.clean) updateTabStatus(tab.id, "disconnected");
        })();
      });
      rfbRef.current = rfb;
    } catch (cause) {
      connectingRef.current = false;
      setPassword("");
      failureRef.current = true;
      setFailure(classifyVncStartFailure(cause));
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

  const approveServer = () => {
    rfbRef.current?.approveServer();
    setServerVerification(undefined);
    connectingRef.current = true;
  };

  const openExternalViewer = async () => {
    await launchVncViewer(host, port).catch(() => {
      failureRef.current = true;
      setFailure({ kind: "proxy", technicalDetails: [t("vnc.externalViewerFailed")] });
    });
  };

  const failureTitle = failure ? t(`vnc.failures.${failure.kind}.title`) : "";
  const failureMessage = failure ? t(`vnc.failures.${failure.kind}.message`) : "";

  return (
    <div className="flex h-full min-h-0 flex-col bg-black">
      <div className="flex h-10 flex-shrink-0 items-center gap-2 border-b border-dock-border bg-dock-surface px-2">
        <MonitorUp size={14} className="text-sky-400" />
        <span className="min-w-0 flex-1 truncate text-[11px] text-dock-text-muted">{host}:{port}</span>
        <button type="button" className="icon-button" title={viewOnly ? t("vnc.enableInput") : t("vnc.viewOnly")} onClick={() => { const next = !viewOnly; setViewOnly(next); if (rfbRef.current) rfbRef.current.viewOnly = next; }}>{viewOnly ? <Eye size={13} /> : <EyeOff size={13} />}</button>
        <button type="button" className="icon-button" title={scaled ? t("vnc.nativeSize") : t("vnc.fitWorkspace")} onClick={() => { const next = !scaled; setScaled(next); if (rfbRef.current) rfbRef.current.scaleViewport = next; }}><Maximize2 size={13} /></button>
        {rfbRef.current ? <button type="button" className="icon-button" title={t("vnc.disconnect")} onClick={() => void disconnect()}><Unplug size={13} /></button> : <button type="button" className="icon-button" title={t("vnc.connect")} onClick={() => void connect()}><PlugZap size={13} /></button>}
        <button type="button" className="icon-button" title={t("vnc.openExternal")} onClick={() => void openExternalViewer()}><ExternalLink size={13} /></button>
      </div>

      <div className="relative min-h-0 flex-1 overflow-hidden">
        <div ref={targetRef} className="h-full w-full overflow-hidden [&>div]:h-full [&_canvas]:max-h-full [&_canvas]:max-w-full" />
        {credentialsRequired && (
          <form onSubmit={submitCredentials} className="absolute inset-0 flex items-center justify-center bg-black/80 p-5">
            <div className="w-full max-w-sm rounded-md border border-dock-border bg-dock-surface p-4">
              <h2 className="mb-3 text-sm font-semibold text-dock-text">{t("vnc.credentials")}</h2>
              <label className="mb-2 block text-[11px] text-dock-text-muted">{t("vnc.username")}<input autoComplete="username" value={username} onChange={(event) => setUsername(event.target.value)} className="form-input mt-1 w-full" /></label>
              <label className="block text-[11px] text-dock-text-muted">{t("vnc.password")}<input autoFocus required autoComplete="current-password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} className="form-input mt-1 w-full" /></label>
              <button className="mt-3 w-full rounded-md bg-dock-accent px-3 py-2 text-xs font-medium text-white">{t("vnc.connect")}</button>
            </div>
          </form>
        )}
        {serverVerification && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/80 p-5">
            <div className="w-full max-w-lg rounded-md border border-amber-500/40 bg-dock-surface p-4 text-xs">
              <div className="mb-2 flex items-center gap-2 text-amber-400"><ShieldCheck size={16} /><h2 className="font-semibold">{t("vnc.verifyServer")}</h2></div>
              <p className="mb-3 text-dock-text-muted">{t("vnc.verifyServerMessage")}</p>
              <code dir="ltr" className="block break-all rounded bg-black/30 p-2 text-[11px] text-dock-text">SHA-256 {serverVerification.fingerprint}</code>
              <div className="mt-3 flex justify-end gap-2">
                <button type="button" className="rounded border border-dock-border px-3 py-1.5 text-dock-text" onClick={() => void disconnect()}>{t("vnc.cancel")}</button>
                <button type="button" className="rounded bg-dock-accent px-3 py-1.5 font-medium text-white" onClick={approveServer}>{t("vnc.trustOnce")}</button>
              </div>
            </div>
          </div>
        )}
        {failure && (
          <div className="absolute inset-x-4 bottom-4 rounded-md border border-red-500/30 bg-dock-surface p-3 text-xs text-dock-text shadow-lg">
            <div className="flex items-start gap-3">
              <AlertTriangle size={16} className="mt-0.5 flex-none text-dock-error" />
              <div className="min-w-0 flex-1"><p className="font-semibold text-dock-error">{failureTitle}</p><p className="mt-1 text-dock-text-muted">{failureMessage}</p></div>
              <button className="rounded bg-dock-accent px-2.5 py-1.5 text-white" onClick={() => void connect()}>{t("vnc.reconnect")}</button>
              {failure.externalViewerRecommended && <button className="flex items-center gap-1 rounded border border-dock-border px-2.5 py-1.5 text-dock-text" onClick={() => void openExternalViewer()}><ExternalLink size={12} />{t("vnc.openExternal")}</button>}
            </div>
            {failure.technicalDetails && failure.technicalDetails.length > 0 && <details className="mt-2 ps-7 text-[11px] text-dock-text-muted"><summary className="cursor-pointer">{t("vnc.technicalDetails")}</summary><div dir="ltr" className="mt-1 space-y-0.5 font-mono">{failure.technicalDetails.map((detail) => <div key={detail}>{detail}</div>)}</div></details>}
          </div>
        )}
      </div>
    </div>
  );
}