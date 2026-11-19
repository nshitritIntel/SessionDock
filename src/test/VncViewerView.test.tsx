import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "../i18n";
import { VncViewerView } from "../components/vnc/VncViewerView";
import { useAppStore, type TerminalTab } from "../stores/appStore";
import type { Session } from "../types";

const mocks = vi.hoisted(() => ({ approveServer: vi.fn(), instances: [] as EventTarget[], invoke: vi.fn(), rfb: vi.fn() }));

vi.mock("../api/native", () => ({ nativeInvoke: mocks.invoke }));
vi.mock("zustand/middleware", () => ({ persist: (initializer: unknown) => initializer }));
vi.mock("@novnc/novnc", () => ({
  default: class MockRfb extends EventTarget {
    viewOnly = false;
    focusOnClick = true;
    clipViewport = false;
    dragViewport = false;
    scaleViewport = false;
    resizeSession = false;
    showDotCursor = false;
    qualityLevel = 0;
    compressionLevel = 0;
    constructor(target: Element, url: string, options?: unknown) {
      super();
      mocks.rfb(target, url, options);
      mocks.instances.push(this);
    }
    approveServer() { mocks.approveServer(); }
    disconnect() {}
    focus() {}
    sendCredentials() {}
  },
}));

const tab: TerminalTab = {
  id: "vnc-tab",
  sessionId: "vnc-session",
  sessionName: "VM console",
  host: "192.0.2.25",
  port: 5901,
  protocol: "vnc",
  status: "connecting",
  pinned: false,
};

describe("VncViewerView", () => {
  beforeEach(() => {
    mocks.invoke.mockReset().mockImplementation((command: string) => {
      if (command === "start_vnc_proxy") {
        return Promise.resolve({ id: "33333333-3333-4333-8333-333333333333", url: "ws://127.0.0.1:49152/vnc/token" });
      }
      if (command === "get_vnc_proxy_diagnostics") {
        return Promise.resolve({
          stage: "authentication",
          serverProtocolVersion: "003.008",
          offeredSecurityTypes: [2],
          selectedSecurityType: 2,
        });
      }
      return Promise.resolve(undefined);
    });
    mocks.approveServer.mockReset();
    mocks.instances.length = 0;
    mocks.rfb.mockReset();
    useAppStore.setState({ tabs: [tab], activeTabId: tab.id, currentView: "terminal" });
  });

  it("owns the native proxy for the lifetime of the VNC tab", async () => {
    const view = render(<VncViewerView tab={tab} active />);

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("start_vnc_proxy", {
      host: "192.0.2.25",
      port: 5901,
    }));
    expect(mocks.rfb).toHaveBeenCalledWith(
      expect.any(HTMLDivElement),
      "ws://127.0.0.1:49152/vnc/token",
      { shared: true, credentials: undefined },
    );

    view.unmount();
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("stop_vnc_proxy", {
      id: "33333333-3333-4333-8333-333333333333",
    }));
  });

  it("uses the current saved endpoint instead of stale tab metadata", async () => {
    const session = {
      id: "vnc-session",
      name: "VM console",
      protocol: "vnc",
      host: "192.0.2.80",
      port: 5780,
    } as Session;

    render(<VncViewerView tab={tab} session={session} active />);

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("start_vnc_proxy", {
      host: "192.0.2.80",
      port: 5780,
    }));
  });

  it("removes a quick-connect password after handing it to noVNC", async () => {
    const passwordTab = { ...tab, password: "temporary-secret" };
    useAppStore.setState({ tabs: [passwordTab] });

    render(<VncViewerView tab={passwordTab} active />);

    await waitFor(() => expect(mocks.rfb).toHaveBeenCalledWith(
      expect.any(HTMLDivElement),
      "ws://127.0.0.1:49152/vnc/token",
      { shared: true, credentials: { username: "", password: "temporary-secret" } },
    ));
    expect(useAppStore.getState().tabs[0].password).toBeUndefined();
  });

  it("classifies mandatory encrypted RealVNC security and offers the compatible viewer", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "start_vnc_proxy") return Promise.resolve({ id: "33333333-3333-4333-8333-333333333333", url: "ws://127.0.0.1:49152/vnc/token" });
      if (command === "get_vnc_proxy_diagnostics") return Promise.resolve({
        stage: "security_negotiation",
        serverProtocolVersion: "003.008",
        offeredSecurityTypes: [13, 133, 5, 129],
      });
      return Promise.resolve(undefined);
    });
    render(<VncViewerView tab={tab} active />);
    await waitFor(() => expect(mocks.instances).toHaveLength(1));

    mocks.instances[0].dispatchEvent(new CustomEvent("disconnect", { detail: { clean: false } }));

    expect(await screen.findByText("Embedded viewer could not establish encryption")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Open in RealVNC Viewer/i })).toHaveLength(2);
    fireEvent.click(screen.getByText("Technical details"));
    expect(screen.getByText("Server security types: Unknown (13), Unknown (133), RA2 (5), RA2_256 (129)")).toBeInTheDocument();
  });

  it("does not report RA2_256 as unsupported after it was selected", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "start_vnc_proxy") return Promise.resolve({ id: "44444444-4444-4444-8444-444444444444", url: "ws://127.0.0.1:49152/vnc/token" });
      if (command === "get_vnc_proxy_diagnostics") return Promise.resolve({
        stage: "authentication",
        serverProtocolVersion: "005.000",
        offeredSecurityTypes: [13, 133, 5, 129],
        selectedSecurityType: 129,
      });
      return Promise.resolve(undefined);
    });
    render(<VncViewerView tab={tab} active />);
    await waitFor(() => expect(mocks.instances).toHaveLength(1));

    mocks.instances[0].dispatchEvent(new CustomEvent("disconnect", { detail: { clean: false } }));

    expect(await screen.findByText("VNC server disconnected")).toBeInTheDocument();
    expect(screen.queryByText("Embedded viewer could not establish encryption")).not.toBeInTheDocument();
  });

  it("preserves a supported authentication failure instead of replacing it on disconnect", async () => {
    render(<VncViewerView tab={tab} active />);
    await waitFor(() => expect(mocks.instances).toHaveLength(1));

    mocks.instances[0].dispatchEvent(new CustomEvent("securityfailure", { detail: { status: 1, reason: "rejected" } }));
    mocks.instances[0].dispatchEvent(new CustomEvent("disconnect", { detail: { clean: false } }));

    expect(await screen.findByText("VNC authentication failed")).toBeInTheDocument();
    expect(screen.queryByText("VNC server disconnected")).not.toBeInTheDocument();
  });

  it("requires explicit approval of the RSA server fingerprint", async () => {
    render(<VncViewerView tab={tab} active />);
    await waitFor(() => expect(mocks.instances).toHaveLength(1));

    mocks.instances[0].dispatchEvent(new CustomEvent("serververification", {
      detail: { type: "RSA", publickey: new Uint8Array([1, 2, 3, 4]) },
    }));

    const approve = await screen.findByRole("button", { name: "Trust for this connection" });
    expect(screen.getByText(/SHA-256/)).toBeInTheDocument();
    fireEvent.click(approve);
    expect(mocks.approveServer).toHaveBeenCalledOnce();
  });

  it("reports a refused TCP connection without creating an RFB client", async () => {
    mocks.invoke.mockImplementation((command: string) => command === "start_vnc_proxy"
      ? Promise.reject(new Error("VNC_TCP_REFUSED: refused"))
      : Promise.resolve(undefined));

    render(<VncViewerView tab={tab} active />);

    expect(await screen.findByText("VNC connection refused")).toBeInTheDocument();
    expect(mocks.instances).toHaveLength(0);
  });
});
