import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const allowedAdvisory = {
  package: "picomatch",
  source: 1247710,
  url: "https://github.com/advisories/GHSA-fc5q-39xc-vjpf",
  expires: "2026-12-18T00:00:00Z",
};
const severityRank = { info: 0, low: 1, moderate: 2, high: 3, critical: 4 };

function runAudit(args) {
  const npmCli = process.env.npm_execpath;
  if (!npmCli)
    throw new Error("npm_execpath is unavailable; run this check through npm");
  const result = spawnSync(
    process.execPath,
    [npmCli, "audit", "--json", ...args],
    {
      encoding: "utf8",
    },
  );
  if (result.error) throw result.error;
  try {
    return { report: JSON.parse(result.stdout), status: result.status };
  } catch {
    process.stderr.write(result.stderr || result.stdout);
    throw new Error("npm audit did not return valid JSON");
  }
}

const production = runAudit(["--omit=dev", "--audit-level=high"]);
if (production.status !== 0) {
  console.error("Production dependency audit failed.");
  process.exit(production.status ?? 1);
}

const full = runAudit(["--audit-level=high"]);
if (full.status === 0) {
  console.log("npm audit passed with no high or critical vulnerabilities.");
  process.exit(0);
}

if (Date.now() >= Date.parse(allowedAdvisory.expires)) {
  console.error(`Audit exception expired at ${allowedAdvisory.expires}.`);
  process.exit(1);
}

const vulnerabilities = full.report.vulnerabilities ?? {};
const highVulnerabilities = Object.entries(vulnerabilities).filter(
  ([, vulnerability]) =>
    severityRank[vulnerability.severity] >= severityRank.high,
);
const resolvesOnlyToAllowedAdvisory = (name, visited = new Set()) => {
  if (visited.has(name)) return false;
  const vulnerability = vulnerabilities[name];
  if (!vulnerability) return false;
  const nextVisited = new Set(visited).add(name);
  return (
    vulnerability.via.length > 0 &&
    vulnerability.via.every((via) => {
      if (typeof via === "string")
        return resolvesOnlyToAllowedAdvisory(via, nextVisited);
      return (
        via.source === allowedAdvisory.source &&
        via.name === allowedAdvisory.package &&
        via.url === allowedAdvisory.url
      );
    })
  );
};

if (
  !highVulnerabilities.every(([name]) => resolvesOnlyToAllowedAdvisory(name))
) {
  console.error(
    "npm audit found a high or critical vulnerability outside the approved exception.",
  );
  process.exit(1);
}

const lockfile = JSON.parse(
  readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"),
);
const allowedNodes = vulnerabilities[allowedAdvisory.package]?.nodes ?? [];
if (
  allowedNodes.length === 0 ||
  allowedNodes.some((node) => lockfile.packages?.[node]?.dev !== true)
) {
  console.error(
    "The approved advisory is not confined to dev-only lockfile nodes.",
  );
  process.exit(1);
}

console.warn(
  `Accepted dev-only ${allowedAdvisory.url} until ${allowedAdvisory.expires}; no patched release exists.`,
);
