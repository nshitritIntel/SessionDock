import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Activity,
  BellRing,
  CalendarClock,
  Check,
  GitCompare,
  Pause,
  Play,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import {
  acknowledgeMonitorIncident,
  captureMonitorBaseline,
  compareMonitorBaseline,
  createMonitorDefinition,
  createMonitorMaintenance,
  deleteMonitorBaseline,
  deleteMonitorDefinition,
  deleteMonitorMaintenance,
  getMonitorBaselines,
  getMonitorDefinitions,
  getMonitorIncidents,
  getMonitorMaintenance,
  getMonitorSamples,
  runMonitorNow,
  setMonitorEnabled,
  type BaselineComparison,
  type MaintenanceMarker,
  type MonitorBaseline,
  type MonitorDefinition,
  type MonitorIncident,
  type MonitorSample,
} from "../../api/commands";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";

type DashboardTab = "overview" | "incidents" | "baselines" | "maintenance";

export function NetworkHealthView() {
  const { t } = useTranslation();
  const sessions = useSessionStore((state) => state.sessions);
  const addToast = useToastStore((state) => state.addToast);
  const [monitors, setMonitors] = useState<MonitorDefinition[]>([]);
  const [incidents, setIncidents] = useState<MonitorIncident[]>([]);
  const [samples, setSamples] = useState<MonitorSample[]>([]);
  const [baselines, setBaselines] = useState<MonitorBaseline[]>([]);
  const [maintenance, setMaintenance] = useState<MaintenanceMarker[]>([]);
  const [selectedMonitorId, setSelectedMonitorId] = useState<string>();
  const [activeTab, setActiveTab] = useState<DashboardTab>("overview");
  const [loading, setLoading] = useState(true);
  const [runningId, setRunningId] = useState<string>();
  const [setupOpen, setSetupOpen] = useState(false);
  const [comparison, setComparison] = useState<BaselineComparison>();

  const selectedMonitor = monitors.find((monitor) => monitor.id === selectedMonitorId);
  const sshSessions = sessions.filter((session) => session.protocol === "ssh");

  const refreshOverview = async () => {
    const [nextMonitors, nextIncidents, nextMaintenance] = await Promise.all([
      getMonitorDefinitions(),
      getMonitorIncidents(true),
      getMonitorMaintenance(),
    ]);
    setMonitors(nextMonitors);
    setIncidents(nextIncidents);
    setMaintenance(nextMaintenance);
    setSelectedMonitorId((current) =>
      current && nextMonitors.some((monitor) => monitor.id === current)
        ? current
        : nextMonitors[0]?.id,
    );
  };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void refreshOverview()
      .catch((error) => {
        if (!cancelled) addToast("error", `${t("monitoring.loadFailed")}: ${String(error)}`);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [addToast, t]);

  useEffect(() => {
    if (!selectedMonitorId) {
      setSamples([]);
      setBaselines([]);
      return;
    }
    let cancelled = false;
    void Promise.all([
      getMonitorSamples(selectedMonitorId, 48),
      getMonitorBaselines(selectedMonitorId),
    ]).then(([nextSamples, nextBaselines]) => {
      if (!cancelled) {
        setSamples(nextSamples);
        setBaselines(nextBaselines);
      }
    }).catch((error) => {
      if (!cancelled) addToast("error", `${t("monitoring.loadFailed")}: ${String(error)}`);
    });
    return () => { cancelled = true; };
  }, [addToast, selectedMonitorId, t]);

  const refresh = async () => {
    try {
      await refreshOverview();
      if (selectedMonitorId) {
        const [nextSamples, nextBaselines] = await Promise.all([
          getMonitorSamples(selectedMonitorId, 48),
          getMonitorBaselines(selectedMonitorId),
        ]);
        setSamples(nextSamples);
        setBaselines(nextBaselines);
      }
    } catch (error) {
      addToast("error", `${t("monitoring.loadFailed")}: ${String(error)}`);
    }
  };

  const runNow = async (id: string) => {
    setRunningId(id);
    try {
      await runMonitorNow(id);
      await refresh();
      addToast("success", t("monitoring.runComplete"));
    } catch (error) {
      await refresh();
      addToast("error", `${t("monitoring.runFailed")}: ${String(error)}`);
    } finally {
      setRunningId(undefined);
    }
  };

  const toggleMonitor = async (monitor: MonitorDefinition) => {
    try {
      const updated = await setMonitorEnabled(monitor.id, !monitor.enabled);
      setMonitors((current) => current.map((item) => item.id === updated.id ? updated : item));
    } catch (error) {
      addToast("error", `${t("monitoring.updateFailed")}: ${String(error)}`);
    }
  };

  const removeMonitor = async (monitor: MonitorDefinition) => {
    if (!window.confirm(t("monitoring.deleteConfirm", { name: monitor.displayName }))) return;
    try {
      await deleteMonitorDefinition(monitor.id);
      await refreshOverview();
    } catch (error) {
      addToast("error", `${t("monitoring.deleteFailed")}: ${String(error)}`);
    }
  };

  const statusCounts = monitors.reduce<Record<string, number>>((counts, monitor) => {
    const status = monitor.latestStatus ?? "unknown";
    counts[status] = (counts[status] ?? 0) + 1;
    return counts;
  }, {});
  const openIncidents = incidents.filter((incident) => incident.state !== "resolved");

  return (
    <div className="h-full overflow-y-auto bg-dock-bg">
      <header className="sticky top-0 z-20 flex min-h-14 items-center gap-3 border-b border-dock-border bg-dock-bg/95 px-5 backdrop-blur-sm">
        <Activity size={18} className="text-dock-accent" />
        <div className="min-w-0 flex-1">
          <h1 className="text-sm font-semibold text-dock-text">{t("monitoring.title")}</h1>
          <p className="text-[11px] text-dock-text-muted">{t("monitoring.subtitle")}</p>
        </div>
        <button className="icon-button" title={t("common.refresh")} onClick={() => void refresh()}>
          <RefreshCw size={15} />
        </button>
        <button
          className="flex items-center gap-1.5 rounded-md bg-dock-accent px-3 py-2 text-xs font-medium text-white hover:bg-dock-accent-hover"
          onClick={() => setSetupOpen(true)}
        >
          <Plus size={14} /> {t("monitoring.addMonitor")}
        </button>
      </header>

      {setupOpen && (
        <MonitorSetup
          sessions={sshSessions}
          onClose={() => setSetupOpen(false)}
          onCreated={async (monitor) => {
            setSetupOpen(false);
            await refreshOverview();
            setSelectedMonitorId(monitor.id);
          }}
        />
      )}

      <div className="mx-auto max-w-7xl px-5 py-5">
        <div className="mb-5 grid grid-cols-2 gap-px overflow-hidden rounded-md border border-dock-border bg-dock-border lg:grid-cols-5">
          <HealthMetric label={t("monitoring.monitors")} value={monitors.length} tone="neutral" />
          <HealthMetric label={t("monitoring.healthy")} value={statusCounts.healthy ?? 0} tone="healthy" />
          <HealthMetric label={t("monitoring.attention")} value={(statusCounts.warning ?? 0) + (statusCounts.critical ?? 0) + (statusCounts.failed ?? 0)} tone="warning" />
          <HealthMetric label={t("monitoring.openIncidents")} value={openIncidents.length} tone="critical" />
          <HealthMetric label={t("monitoring.maintenance")} value={maintenance.filter(isActiveMaintenance).length} tone="neutral" />
        </div>

        <div className="mb-4 flex gap-1 border-b border-dock-border">
          {(["overview", "incidents", "baselines", "maintenance"] as DashboardTab[]).map((tab) => (
            <button
              key={tab}
              onClick={() => setActiveTab(tab)}
              className={`border-b-2 px-3 py-2 text-xs font-medium ${activeTab === tab ? "border-dock-accent text-dock-accent" : "border-transparent text-dock-text-muted hover:text-dock-text"}`}
            >
              {t(`monitoring.tabs.${tab}`)}
            </button>
          ))}
        </div>

        {loading ? (
          <div className="py-16 text-center text-xs text-dock-text-muted">{t("common.loading")}</div>
        ) : activeTab === "overview" ? (
          <Overview
            monitors={monitors}
            selectedId={selectedMonitorId}
            samples={samples}
            runningId={runningId}
            onSelect={setSelectedMonitorId}
            onRun={runNow}
            onToggle={toggleMonitor}
            onDelete={removeMonitor}
          />
        ) : activeTab === "incidents" ? (
          <Incidents
            incidents={incidents}
            monitorNames={new Map(monitors.map((monitor) => [monitor.id, monitor.displayName]))}
            onAcknowledge={async (id) => {
              try {
                await acknowledgeMonitorIncident(id);
                await refreshOverview();
              } catch (error) {
                addToast("error", `${t("monitoring.updateFailed")}: ${String(error)}`);
              }
            }}
          />
        ) : activeTab === "baselines" ? (
          <Baselines
            monitor={selectedMonitor}
            monitors={monitors}
            baselines={baselines}
            comparison={comparison}
            onSelectMonitor={setSelectedMonitorId}
            onCapture={async () => {
              if (!selectedMonitorId) return;
              const name = window.prompt(t("monitoring.baselineNamePrompt"))?.trim();
              if (!name) return;
              try {
                await captureMonitorBaseline(selectedMonitorId, name);
                setBaselines(await getMonitorBaselines(selectedMonitorId));
              } catch (error) {
                addToast("error", `${t("monitoring.baselineFailed")}: ${String(error)}`);
              }
            }}
            onCompare={async (id) => {
              try {
                setComparison(await compareMonitorBaseline(id));
              } catch (error) {
                addToast("error", `${t("monitoring.compareFailed")}: ${String(error)}`);
              }
            }}
            onDelete={async (id) => {
              await deleteMonitorBaseline(id);
              if (selectedMonitorId) setBaselines(await getMonitorBaselines(selectedMonitorId));
              setComparison(undefined);
            }}
          />
        ) : (
          <Maintenance
            markers={maintenance}
            sessions={sessions}
            onCreate={async (request) => {
              try {
                await createMonitorMaintenance(request);
                setMaintenance(await getMonitorMaintenance());
              } catch (error) {
                addToast("error", `${t("monitoring.maintenanceFailed")}: ${String(error)}`);
              }
            }}
            onDelete={async (id) => {
              await deleteMonitorMaintenance(id);
              setMaintenance(await getMonitorMaintenance());
            }}
          />
        )}
      </div>
    </div>
  );
}

function MonitorSetup({ sessions, onClose, onCreated }: {
  sessions: ReturnType<typeof useSessionStore.getState>["sessions"];
  onClose: () => void;
  onCreated: (monitor: MonitorDefinition) => Promise<void>;
}) {
  const { t } = useTranslation();
  const addToast = useToastStore((state) => state.addToast);
  const [sessionId, setSessionId] = useState(sessions[0]?.id ?? "");
  const [name, setName] = useState("");
  const [intervalSeconds, setIntervalSeconds] = useState(300);
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setSubmitting(true);
    try {
      const session = sessions.find((item) => item.id === sessionId);
      const monitor = await createMonitorDefinition({
        sessionId,
        providerId: "arista-eos",
        monitorKind: "interfaces",
        displayName: name.trim() || `${session?.name ?? "Arista"} interfaces`,
        intervalSeconds,
        retentionHours: 168,
      });
      await onCreated(monitor);
    } catch (error) {
      addToast("error", `${t("monitoring.createFailed")}: ${String(error)}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form onSubmit={submit} className="border-b border-dock-border bg-dock-surface px-5 py-4">
      <div className="mx-auto grid max-w-7xl items-end gap-3 sm:grid-cols-[minmax(12rem,1fr)_minmax(12rem,1fr)_10rem_auto]">
        <label className="text-[11px] text-dock-text-muted">
          {t("monitoring.sshSession")}
          <select required value={sessionId} onChange={(event) => setSessionId(event.target.value)} className="form-input mt-1 w-full">
            <option value="">{t("monitoring.selectSession")}</option>
            {sessions.map((session) => <option key={session.id} value={session.id}>{session.name} · {session.host}</option>)}
          </select>
        </label>
        <label className="text-[11px] text-dock-text-muted">
          {t("monitoring.monitorName")}
          <input value={name} onChange={(event) => setName(event.target.value)} maxLength={100} className="form-input mt-1 w-full" />
        </label>
        <label className="text-[11px] text-dock-text-muted">
          {t("monitoring.interval")}
          <select value={intervalSeconds} onChange={(event) => setIntervalSeconds(Number(event.target.value))} className="form-input mt-1 w-full">
            <option value={60}>1 min</option><option value={300}>5 min</option><option value={900}>15 min</option><option value={3600}>1 hour</option>
          </select>
        </label>
        <div className="flex gap-2">
          <button type="button" onClick={onClose} className="icon-button" title={t("common.cancel")}><X size={15} /></button>
          <button disabled={submitting || !sessionId} className="rounded-md bg-dock-accent px-3 py-2 text-xs font-medium text-white disabled:opacity-50">{t("common.create")}</button>
        </div>
      </div>
    </form>
  );
}

function HealthMetric({ label, value, tone }: { label: string; value: number; tone: "neutral" | "healthy" | "warning" | "critical" }) {
  const tones = { neutral: "text-dock-text", healthy: "text-emerald-400", warning: "text-amber-400", critical: "text-red-400" };
  return <div className="bg-dock-surface px-4 py-3"><div className={`text-xl font-semibold tabular-nums ${tones[tone]}`}>{value}</div><div className="text-[11px] text-dock-text-muted">{label}</div></div>;
}

function Overview({ monitors, selectedId, samples, runningId, onSelect, onRun, onToggle, onDelete }: {
  monitors: MonitorDefinition[]; selectedId?: string; samples: MonitorSample[]; runningId?: string;
  onSelect: (id: string) => void; onRun: (id: string) => Promise<void>;
  onToggle: (monitor: MonitorDefinition) => Promise<void>; onDelete: (monitor: MonitorDefinition) => Promise<void>;
}) {
  const { t } = useTranslation();
  if (monitors.length === 0) return <Empty icon={Activity} text={t("monitoring.noMonitors")} />;
  return <div className="grid gap-5 xl:grid-cols-[minmax(30rem,1fr)_22rem]">
    <div className="overflow-hidden rounded-md border border-dock-border">
      <table className="w-full text-left text-xs"><thead className="bg-dock-surface text-[10px] uppercase text-dock-text-muted"><tr><th className="px-3 py-2">{t("monitoring.device")}</th><th>{t("monitoring.status")}</th><th>{t("monitoring.lastSample")}</th><th className="w-28" /></tr></thead>
      <tbody>{monitors.map((monitor) => <tr key={monitor.id} onClick={() => onSelect(monitor.id)} className={`cursor-pointer border-t border-dock-border ${selectedId === monitor.id ? "bg-dock-accent/5" : "hover:bg-dock-surface"}`}><td className="px-3 py-2.5"><div className="font-medium text-dock-text">{monitor.displayName}</div><div className="text-[10px] text-dock-text-muted">{monitor.sessionName} · {monitor.host}</div></td><td><Status value={monitor.latestStatus ?? "unknown"} /></td><td className="text-dock-text-muted">{formatTime(monitor.latestObservedAt)}</td><td><div className="flex justify-end gap-1 pr-2"><button className="icon-button" title={monitor.enabled ? t("monitoring.pause") : t("monitoring.enable")} onClick={(event) => { event.stopPropagation(); void onToggle(monitor); }}>{monitor.enabled ? <Pause size={13} /> : <Play size={13} />}</button><button className="icon-button" title={t("monitoring.runNow")} disabled={runningId === monitor.id || !monitor.enabled} onClick={(event) => { event.stopPropagation(); void onRun(monitor.id); }}><RefreshCw size={13} className={runningId === monitor.id ? "animate-spin" : ""} /></button><button className="icon-button text-red-400" title={t("common.delete")} onClick={(event) => { event.stopPropagation(); void onDelete(monitor); }}><Trash2 size={13} /></button></div></td></tr>)}</tbody></table>
    </div>
    <div><h2 className="mb-3 text-xs font-medium text-dock-text">{t("monitoring.recentHealth")}</h2><History samples={samples} /></div>
  </div>;
}

function History({ samples }: { samples: MonitorSample[] }) {
  const { t } = useTranslation();
  if (samples.length === 0) return <Empty icon={Activity} text={t("monitoring.noSamples")} />;
  return <div className="space-y-1">{samples.slice(0, 16).map((sample) => <div key={sample.id} className="grid grid-cols-[5rem_1fr_auto] items-center gap-2 py-1.5"><span className="text-[10px] text-dock-text-muted">{formatTime(sample.observedAt)}</span><div className="h-1.5 overflow-hidden rounded-sm bg-dock-border"><div className={`h-full ${statusBar(sample.status)}`} style={{ width: `${Math.max(8, 100 - (sample.summary.downCount ?? 0) * 10)}%` }} /></div><Status value={sample.status} /></div>)}</div>;
}

function Incidents({ incidents, monitorNames, onAcknowledge }: { incidents: MonitorIncident[]; monitorNames: Map<string, string>; onAcknowledge: (id: string) => Promise<void> }) {
  const { t } = useTranslation();
  if (incidents.length === 0) return <Empty icon={ShieldCheck} text={t("monitoring.noIncidents")} />;
  return <div className="divide-y divide-dock-border border-y border-dock-border">{incidents.map((incident) => <div key={incident.id} className="grid gap-2 py-3 sm:grid-cols-[1fr_auto] sm:items-center"><div><div className="flex items-center gap-2"><BellRing size={13} className={incident.state === "resolved" ? "text-dock-text-muted" : "text-red-400"} /><span className="text-xs font-medium text-dock-text">{incident.title}</span><Status value={incident.state} /></div><div className="mt-1 text-[11px] text-dock-text-muted">{monitorNames.get(incident.monitorId)} · {incident.details} · {t("monitoring.occurrences", { count: incident.occurrenceCount })}</div></div>{incident.state === "open" && <button onClick={() => void onAcknowledge(incident.id)} className="flex items-center gap-1 rounded-md border border-dock-border px-2.5 py-1.5 text-[11px] text-dock-text-secondary hover:bg-dock-surface"><Check size={12} />{t("monitoring.acknowledge")}</button>}</div>)}</div>;
}

function Baselines({ monitor, monitors, baselines, comparison, onSelectMonitor, onCapture, onCompare, onDelete }: {
  monitor?: MonitorDefinition; monitors: MonitorDefinition[]; baselines: MonitorBaseline[]; comparison?: BaselineComparison;
  onSelectMonitor: (id: string) => void; onCapture: () => Promise<void>; onCompare: (id: string) => Promise<void>; onDelete: (id: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  return <div><div className="mb-4 flex flex-wrap items-center gap-2"><select value={monitor?.id ?? ""} onChange={(event) => onSelectMonitor(event.target.value)} className="form-input min-w-56">{monitors.map((item) => <option key={item.id} value={item.id}>{item.displayName}</option>)}</select><button disabled={!monitor} onClick={() => void onCapture()} className="flex items-center gap-1.5 rounded-md bg-dock-accent px-3 py-2 text-xs text-white disabled:opacity-50"><Plus size={13} />{t("monitoring.captureBaseline")}</button></div>{baselines.length === 0 ? <Empty icon={GitCompare} text={t("monitoring.noBaselines")} /> : <div className="divide-y divide-dock-border border-y border-dock-border">{baselines.map((baseline) => <div key={baseline.id} className="flex items-center gap-3 py-3"><GitCompare size={14} className="text-dock-text-muted" /><div className="min-w-0 flex-1"><div className="text-xs font-medium text-dock-text">{baseline.name}</div><div className="text-[10px] text-dock-text-muted">{formatTime(baseline.sourceObservedAt)}</div></div><button className="icon-button" title={t("monitoring.compare")} onClick={() => void onCompare(baseline.id)}><GitCompare size={13} /></button><button className="icon-button text-red-400" title={t("common.delete")} onClick={() => void onDelete(baseline.id)}><Trash2 size={13} /></button></div>)}</div>}{comparison && <div className="mt-5 grid grid-cols-3 gap-px overflow-hidden rounded-md border border-dock-border bg-dock-border"><Diff label={t("monitoring.added")} values={comparison.addedInterfaces} tone="text-emerald-400" /><Diff label={t("monitoring.removed")} values={comparison.removedInterfaces} tone="text-red-400" /><Diff label={t("monitoring.changed")} values={comparison.changedInterfaces} tone="text-amber-400" /></div>}</div>;
}

function Maintenance({ markers, sessions, onCreate, onDelete }: { markers: MaintenanceMarker[]; sessions: ReturnType<typeof useSessionStore.getState>["sessions"]; onCreate: (request: { sessionId?: string; startsAt: string; endsAt: string; reason: string }) => Promise<void>; onDelete: (id: string) => Promise<void> }) {
  const { t } = useTranslation();
  const [sessionId, setSessionId] = useState(""); const [startsAt, setStartsAt] = useState(""); const [endsAt, setEndsAt] = useState(""); const [reason, setReason] = useState("");
  const submit = async (event: React.FormEvent) => { event.preventDefault(); await onCreate({ sessionId: sessionId || undefined, startsAt: new Date(startsAt).toISOString(), endsAt: new Date(endsAt).toISOString(), reason }); setReason(""); };
  return <div><form onSubmit={submit} className="mb-5 grid gap-2 border-b border-dock-border pb-5 lg:grid-cols-[1fr_1fr_1fr_1.5fr_auto]"><select value={sessionId} onChange={(event) => setSessionId(event.target.value)} className="form-input"><option value="">{t("monitoring.allDevices")}</option>{sessions.map((session) => <option key={session.id} value={session.id}>{session.name}</option>)}</select><input required type="datetime-local" value={startsAt} onChange={(event) => setStartsAt(event.target.value)} className="form-input" /><input required type="datetime-local" value={endsAt} onChange={(event) => setEndsAt(event.target.value)} className="form-input" /><input required maxLength={500} value={reason} onChange={(event) => setReason(event.target.value)} placeholder={t("monitoring.reason")} className="form-input" /><button className="rounded-md bg-dock-accent px-3 py-2 text-xs text-white">{t("common.create")}</button></form>{markers.length === 0 ? <Empty icon={CalendarClock} text={t("monitoring.noMaintenance")} /> : <div className="divide-y divide-dock-border">{markers.map((marker) => <div key={marker.id} className="flex items-center gap-3 py-3"><CalendarClock size={14} className={isActiveMaintenance(marker) ? "text-amber-400" : "text-dock-text-muted"} /><div className="flex-1"><div className="text-xs text-dock-text">{marker.reason}</div><div className="text-[10px] text-dock-text-muted">{formatTime(marker.startsAt)} – {formatTime(marker.endsAt)}</div></div><button className="icon-button text-red-400" title={t("common.delete")} onClick={() => void onDelete(marker.id)}><Trash2 size={13} /></button></div>)}</div>}</div>;
}

function Diff({ label, values, tone }: { label: string; values: string[]; tone: string }) { return <div className="min-h-24 bg-dock-surface p-3"><div className={`mb-2 text-xs font-semibold ${tone}`}>{values.length} {label}</div><div className="space-y-1 text-[10px] text-dock-text-muted">{values.slice(0, 8).map((value) => <div key={value}>{value}</div>)}</div></div>; }
function Empty({ icon: Icon, text }: { icon: typeof Activity; text: string }) { return <div className="py-14 text-center text-dock-text-muted"><Icon size={24} className="mx-auto mb-2 opacity-40" /><p className="text-xs">{text}</p></div>; }
function Status({ value }: { value: string }) { const classes: Record<string, string> = { healthy: "bg-emerald-500/10 text-emerald-400", warning: "bg-amber-500/10 text-amber-400", critical: "bg-red-500/10 text-red-400", failed: "bg-red-500/10 text-red-400", open: "bg-red-500/10 text-red-400", acknowledged: "bg-amber-500/10 text-amber-400", resolved: "bg-dock-border text-dock-text-muted", unknown: "bg-dock-border text-dock-text-muted" }; return <span className={`inline-flex rounded px-1.5 py-0.5 text-[9px] font-medium uppercase ${classes[value] ?? classes.unknown}`}>{value}</span>; }
function statusBar(status: MonitorSample["status"]) { return status === "healthy" ? "bg-emerald-400" : status === "warning" ? "bg-amber-400" : status === "unknown" ? "bg-dock-text-muted" : "bg-red-400"; }
function formatTime(value?: string) { return value ? new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" }).format(new Date(value)) : "—"; }
function isActiveMaintenance(marker: MaintenanceMarker) { const now = Date.now(); return new Date(marker.startsAt).getTime() <= now && new Date(marker.endsAt).getTime() >= now; }
