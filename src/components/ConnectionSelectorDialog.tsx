import { Bookmark, Plug, Server, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { listTerminalPaneIds, useAppStore } from "../stores/appStore";
import { useSessionStore } from "../stores/sessionStore";
import type { Session } from "../types";

export interface ConnectedSshTarget {
  tabId: string;
  paneId: string;
  sessionName: string;
  host: string;
  paneIndex: number;
  paneCount: number;
}

interface ConnectionSelectorDialogProps {
  open: boolean;
  title: string;
  hint: string;
  onClose: () => void;
  onConnected: (target: ConnectedSshTarget) => void;
  onSaved: (session: Session) => void;
  onManual: () => void;
}

export function ConnectionSelectorDialog({
  open,
  title,
  hint,
  onClose,
  onConnected,
  onSaved,
  onManual,
}: ConnectionSelectorDialogProps) {
  const { t } = useTranslation();
  const tabs = useAppStore((state) => state.tabs);
  const sessions = useSessionStore((state) => state.sessions).filter((session) => session.protocol === "ssh");
  const connectedTargets = tabs.flatMap<ConnectedSshTarget>((tab) => {
    if (tab.protocol !== "ssh") return [];
    const paneIds = listTerminalPaneIds(tab.paneLayout ?? { type: "pane", id: tab.id });
    return paneIds.flatMap((paneId, paneIndex) => {
      const status = tab.paneStatuses?.[paneId] ?? tab.status;
      return status === "connected" ? [{
        tabId: tab.id,
        paneId,
        sessionName: tab.sessionName,
        host: tab.host,
        paneIndex,
        paneCount: paneIds.length,
      }] : [];
    });
  });

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={onClose}>
      <section role="dialog" aria-modal="true" aria-labelledby="connection-selector-title" className="w-full max-w-md overflow-hidden rounded border border-dock-border bg-dock-sidebar shadow-xl" onClick={(event) => event.stopPropagation()}>
        <header className="flex items-start gap-2 border-b border-dock-border px-3 py-2.5">
          <Server size={15} className="mt-0.5 text-dock-accent" />
          <div className="min-w-0 flex-1">
            <h3 id="connection-selector-title" className="text-xs font-semibold text-dock-text">{title}</h3>
            <p className="mt-0.5 text-[10px] text-dock-text-muted">{hint}</p>
          </div>
          <button type="button" onClick={onClose} className="p-1 text-dock-text-muted hover:text-dock-text" aria-label={t("session.cancel")}><X size={13} /></button>
        </header>
        <div className="max-h-80 overflow-auto">
          {connectedTargets.length > 0 && (
            <section aria-labelledby="connected-targets-title">
              <h4 id="connected-targets-title" className="px-3 pb-1 pt-2 text-[9px] font-semibold uppercase text-dock-text-muted">{t("connectionSelector.connected")}</h4>
              {connectedTargets.map((target, index) => (
                <button key={`${target.tabId}:${target.paneId}`} type="button" autoFocus={index === 0} onClick={() => onConnected(target)} className="flex w-full items-center gap-2 border-t border-dock-border/50 px-3 py-2 text-start hover:bg-dock-surface focus:bg-dock-surface focus:outline-none">
                  <span className="h-2 w-2 flex-shrink-0 rounded-full bg-emerald-400" />
                  <TargetLabel name={target.sessionName} host={target.host} />
                  {target.paneCount > 1 && <span className="text-[9px] text-dock-text-muted">{t("connectionSelector.pane", { index: target.paneIndex + 1 })}</span>}
                </button>
              ))}
            </section>
          )}
          {sessions.length > 0 && (
            <section aria-labelledby="saved-targets-title">
              <h4 id="saved-targets-title" className="flex items-center gap-1 px-3 pb-1 pt-2 text-[9px] font-semibold uppercase text-dock-text-muted"><Bookmark size={10} />{t("connectionSelector.saved")}</h4>
              {sessions.map((session) => (
                <button key={session.id} type="button" autoFocus={connectedTargets.length === 0 && session === sessions[0]} onClick={() => onSaved(session)} className="flex w-full items-center gap-2 border-t border-dock-border/50 px-3 py-2 text-start hover:bg-dock-surface focus:bg-dock-surface focus:outline-none">
                  <span className="h-2 w-2 flex-shrink-0 rounded-full bg-dock-text-muted" />
                  <TargetLabel name={session.name} host={session.host} />
                </button>
              ))}
            </section>
          )}
          {connectedTargets.length === 0 && sessions.length === 0 && <p className="px-3 py-5 text-center text-xs text-dock-text-muted">{t("connectionSelector.empty")}</p>}
        </div>
        <footer className="border-t border-dock-border p-2">
          <button type="button" onClick={onManual} className="flex h-7 w-full items-center justify-center gap-1.5 rounded border border-dock-border text-[10px] text-dock-text-muted hover:border-dock-accent hover:text-dock-text">
            <Plug size={11} />
            <span>{t("connectionSelector.manual")}</span>
          </button>
        </footer>
      </section>
    </div>
  );
}

function TargetLabel({ name, host }: { name: string; host: string }) {
  return <span className="min-w-0 flex-1"><span className="block truncate text-xs font-medium text-dock-text">{name}</span><span className="block truncate font-mono text-[10px] text-dock-text-muted" dir="ltr">{host}</span></span>;
}