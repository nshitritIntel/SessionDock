import { describe, expect, it } from "vitest";
import { classifyVncDisconnect, classifyVncStartFailure } from "../components/vnc/vncDiagnostics";

describe("VNC diagnostic classification", () => {
  it.each([
    ["VNC_DNS: lookup failed", "dns"],
    ["VNC_TCP_REFUSED: refused", "connection_refused"],
    ["VNC_TCP_TIMEOUT: timed out", "connection_timeout"],
    ["VNC_TCP_BLOCKED: denied", "network"],
    ["VNC_TCP: unreachable", "network"],
    ["VNC_PROXY_LIMIT: busy", "proxy"],
  ] as const)("classifies startup error %s", (error, kind) => {
    expect(classifyVncStartFailure(new Error(error)).kind).toBe(kind);
  });

  it("classifies unknown advertised security types without guessing authentication failure", () => {
    const failure = classifyVncDisconnect({
      stage: "security_negotiation",
      serverProtocolVersion: "003.008",
      offeredSecurityTypes: [200, 201],
    });

    expect(failure.kind).toBe("unsupported_security");
    expect(failure.externalViewerRecommended).toBe(true);
    expect(failure.technicalDetails).toContain("Server security types: Unknown (200), Unknown (201)");
  });

  it("classifies malformed RFB negotiation separately from a remote disconnect", () => {
    expect(classifyVncDisconnect({
      stage: "protocol_version",
      offeredSecurityTypes: [],
      failure: { code: "malformed_protocol", message: "sanitized" },
    }).kind).toBe("protocol");
  });
});