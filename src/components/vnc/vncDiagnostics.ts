import type { VncProxyDiagnostics } from "../../api/commands";

export type VncFailureKind =
  | "authentication"
  | "connection_refused"
  | "connection_timeout"
  | "dns"
  | "encryption"
  | "network"
  | "protocol"
  | "proxy"
  | "remote_disconnect"
  | "unsupported_security";

export interface VncViewerFailure {
  kind: VncFailureKind;
  technicalDetails?: string[];
  externalViewerRecommended?: boolean;
}

const SECURITY_TYPE_NAMES: Record<number, string> = {
  1: "None",
  2: "VNC Authentication",
  5: "RA2",
  6: "RA2ne",
  16: "Tight",
  19: "VeNCrypt",
  22: "XVP",
  30: "Apple Remote Desktop",
  113: "MSLogonII",
  129: "RA2_256",
  130: "RA2ne_256",
};

function securityTypeLabel(securityType: number): string {
  return `${SECURITY_TYPE_NAMES[securityType] ?? "Unknown"} (${securityType})`;
}

function diagnosticDetails(diagnostics?: VncProxyDiagnostics): string[] | undefined {
  if (!diagnostics) return undefined;
  const details = [`Stage: ${diagnostics.stage}`];
  if (diagnostics.serverProtocolVersion) details.push(`RFB version: ${diagnostics.serverProtocolVersion}`);
  if (diagnostics.offeredSecurityTypes.length > 0) {
    details.push(`Server security types: ${diagnostics.offeredSecurityTypes.map(securityTypeLabel).join(", ")}`);
  }
  if (diagnostics.selectedSecurityType !== undefined) {
    details.push(`Selected security type: ${securityTypeLabel(diagnostics.selectedSecurityType)}`);
  }
  if (diagnostics.failure) details.push(`Diagnostic code: ${diagnostics.failure.code}`);
  return details;
}

export function classifyVncStartFailure(cause: unknown): VncViewerFailure {
  const error = String(cause);
  if (error.includes("VNC_DNS")) return { kind: "dns" };
  if (error.includes("VNC_TCP_REFUSED")) return { kind: "connection_refused" };
  if (error.includes("VNC_TCP_TIMEOUT")) return { kind: "connection_timeout" };
  if (error.includes("VNC_TCP_BLOCKED") || error.includes("VNC_TCP:")) return { kind: "network" };
  return { kind: "proxy" };
}

export function classifyVncDisconnect(diagnostics?: VncProxyDiagnostics): VncViewerFailure {
  const technicalDetails = diagnosticDetails(diagnostics);
  const offered = diagnostics?.offeredSecurityTypes ?? [];
  const selected = diagnostics?.selectedSecurityType;
  const encryptedRa2Offered = offered.some((securityType) => securityType === 5 || securityType === 129);
  const encryptedRa2Selected = selected === 5 || selected === 129;
  if (encryptedRa2Offered && !encryptedRa2Selected) {
    return { kind: "encryption", technicalDetails, externalViewerRecommended: true };
  }
  if (diagnostics?.failure?.code === "unsupported_protocol_version" || diagnostics?.failure?.code === "malformed_protocol") {
    return { kind: "protocol", technicalDetails };
  }
  if (diagnostics?.failure?.code === "websocket_handshake_failed") {
    return { kind: "proxy", technicalDetails };
  }
  if (offered.length > 0 && diagnostics?.selectedSecurityType === undefined) {
    return { kind: "unsupported_security", technicalDetails, externalViewerRecommended: true };
  }
  return { kind: "remote_disconnect", technicalDetails };
}