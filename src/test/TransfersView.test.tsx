import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TransfersView } from "../components/views/TransfersView";
import { useAppStore } from "../stores/appStore";
import { useTransferStore } from "../stores/transferStore";
import { useSessionStore } from "../stores/sessionStore";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("../api/native", () => ({ nativeInvoke: mocks.invoke }));
vi.mock("zustand/middleware", () => ({ persist: (initializer: unknown) => initializer }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, values?: { count?: number }) => values ? `${key}:${values.count}` : key }),
}));

describe("TransfersView", () => {
  beforeEach(() => {
    mocks.invoke.mockReset().mockResolvedValue(undefined);
    useAppStore.setState({
      currentView: "transfers",
      activeTabId: null,
      tabs: [{
        id: "tab-1",
        sessionId: "session-1",
        sessionName: "Core switch",
        host: "192.0.2.10",
        protocol: "ssh",
        status: "connected",
        pinned: false,
        paneLayout: { type: "pane", id: "pane-1" },
      }],
    });
    useTransferStore.setState({
      transfers: [{
        id: "transfer-1",
        tabId: "pane-1",
        name: "config.cfg",
        direction: "download",
        transferred: 25,
        total: 100,
        status: "running",
        updatedAt: 1,
      }],
    });
    useSessionStore.setState({ sessions: [] });
  });

  it("shows runtime progress and focuses the owning pane", () => {
    render(<TransfersView />);

    expect(screen.getByText("config.cfg")).toBeInTheDocument();
    expect(screen.getByText("Core switch")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "transfers.focus" }));

    expect(useAppStore.getState()).toMatchObject({ activeTabId: "tab-1", currentView: "terminal" });
    expect(useAppStore.getState().tabs[0].activePaneId).toBe("pane-1");
  });

  it("asks before opening Files on a connected SSH pane", () => {
    render(<TransfersView />);

    fireEvent.click(screen.getByRole("button", { name: "transfers.start" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByText("Core switch"));

    expect(useAppStore.getState()).toMatchObject({ activeTabId: "tab-1", currentView: "terminal" });
    expect(useAppStore.getState().tabs[0]).toMatchObject({
      activePaneId: "pane-1",
      toolPaneId: "pane-1",
      toolPanel: "files",
      toolTransferProtocol: "sftp",
    });
  });

  it("offers Quick Connect when no SSH target exists", () => {
    useAppStore.setState({ tabs: [], activeTabId: null });
    useTransferStore.setState({ transfers: [] });
    const onQuickConnect = vi.fn();
    render(<TransfersView onQuickConnect={onQuickConnect} />);

    fireEvent.click(screen.getAllByRole("button", { name: "transfers.start" })[0]);
    fireEvent.click(screen.getByRole("button", { name: "connectionSelector.manual" }));

    expect(onQuickConnect).toHaveBeenCalledOnce();
  });

  it("opens Files after explicitly selecting a saved SSH session", () => {
    useAppStore.setState({ tabs: [], activeTabId: null });
    useSessionStore.setState({ sessions: [{
      id: "saved-1", name: "Saved leaf", host: "192.0.2.30", port: 22, protocol: "ssh",
      authentication_method: "password", jump_host_ids: [], favorite: false,
      connection_timeout: 30, keepalive_interval: 60, created_at: "2026-01-01",
      updated_at: "2026-01-01", connection_count: 0, bmc_use_https: true,
      bmc_viewer_mode: "web", bmc_ignore_tls_errors: false,
      bmc_open_console_automatically: false, bmc_open_fullscreen: false,
      bmc_timeout_seconds: 30, bmc_redfish_enabled: true, bmc_cookie_persistence: "application",
    }] });
    render(<TransfersView />);

    fireEvent.click(screen.getByRole("button", { name: "transfers.start" }));
    fireEvent.click(screen.getByText("Saved leaf"));

    expect(useAppStore.getState().tabs[0]).toMatchObject({ sessionId: "saved-1", initialAuxiliaryPanel: "files" });
  });

  it("pauses through the native transfer manager", async () => {
    render(<TransfersView />);
    fireEvent.click(screen.getByRole("button", { name: "sftp.pause" }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("pause_sftp_transfer", { transferId: "transfer-1" }));
    expect(useTransferStore.getState().transfers[0].status).toBe("paused");
  });

  it("clears finished history but keeps active work", () => {
    useTransferStore.setState((state) => ({
      transfers: [...state.transfers, {
        id: "transfer-2",
        tabId: "pane-1",
        name: "done.cfg",
        direction: "upload",
        transferred: 100,
        total: 100,
        status: "completed",
        updatedAt: 2,
      }],
    }));
    render(<TransfersView />);
    fireEvent.click(screen.getByRole("button", { name: "transfers.clearFinished" }));

    expect(useTransferStore.getState().transfers.map((transfer) => transfer.id)).toEqual(["transfer-1"]);
  });

  it("retries a failed transfer on its owning pane", async () => {
    useTransferStore.setState({
      transfers: [{
        id: "failed-1",
        tabId: "pane-1",
        name: "config.cfg",
        direction: "upload",
        transferred: 10,
        total: 100,
        status: "failed",
        error: "network failure",
        request: {
          command: "start_sftp_upload",
          args: { tabId: "pane-1", localPath: "C:\\config.cfg", remotePath: "/config.cfg", overwrite: false },
        },
        updatedAt: 1,
      }],
    });
    mocks.invoke.mockResolvedValue("retry-1");
    render(<TransfersView />);
    fireEvent.click(screen.getByRole("button", { name: "sftp.retry" }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("start_sftp_upload", {
      tabId: "pane-1",
      localPath: "C:\\config.cfg",
      remotePath: "/config.cfg",
      overwrite: false,
    }));
    expect(useTransferStore.getState().transfers[0]).toMatchObject({ id: "retry-1", status: "running" });
  });
});