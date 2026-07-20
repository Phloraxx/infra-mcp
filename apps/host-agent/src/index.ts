import { execFile } from "node:child_process";
import { timingSafeEqual } from "node:crypto";
import { chmod, chown, lstat, mkdir, unlink } from "node:fs/promises";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const socketPath = process.env.AGENT_SOCKET ?? "/run/infra-mcp/agent.sock";
const token = process.env.AGENT_TOKEN ?? "";
const socketMode = Number.parseInt(process.env.AGENT_SOCKET_MODE ?? "0660", 8);
const socketGid = process.env.AGENT_SOCKET_GID
  ? Number.parseInt(process.env.AGENT_SOCKET_GID, 10)
  : undefined;

if (token.length < 32) {
  throw new Error("AGENT_TOKEN must be configured with at least 32 characters");
}

const containerNamePattern = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/;
const maxRequestBytes = 64 * 1024;

function json(res: ServerResponse, status: number, value: unknown): void {
  const payload = JSON.stringify(value);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function isAuthorized(req: IncomingMessage): boolean {
  const supplied = req.headers.authorization;
  if (!supplied?.startsWith("Bearer ")) return false;

  const actual = Buffer.from(supplied.slice("Bearer ".length));
  const expected = Buffer.from(token);
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}

async function readJsonBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  let size = 0;

  for await (const chunk of req) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += buffer.length;
    if (size > maxRequestBytes) throw new Error("Request body too large");
    chunks.push(buffer);
  }

  if (chunks.length === 0) return {};
  const parsed: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("JSON body must be an object");
  }
  return parsed as Record<string, unknown>;
}

function requireContainerName(value: unknown): string {
  if (typeof value !== "string" || !containerNamePattern.test(value)) {
    throw new Error("Invalid container name");
  }
  return value;
}

function requireLogLines(value: unknown): number {
  if (value === undefined) return 200;
  if (!Number.isInteger(value) || (value as number) < 1 || (value as number) > 1000) {
    throw new Error("lines must be an integer between 1 and 1000");
  }
  return value as number;
}

async function run(command: string, args: string[], timeout = 15_000) {
  const result = await execFileAsync(command, args, {
    encoding: "utf8",
    timeout,
    maxBuffer: 10 * 1024 * 1024,
  });
  return { stdout: result.stdout, stderr: result.stderr };
}

async function systemStatus() {
  const { stdout } = await run("df", ["-P", "-B1", "/"]);
  const lines = stdout.trim().split("\n");
  const fields = lines.at(-1)?.trim().split(/\s+/) ?? [];
  const [filesystem, totalBytes, usedBytes, availableBytes, usedPercent, mountPoint] = fields;

  return {
    hostname: os.hostname(),
    platform: os.platform(),
    release: os.release(),
    uptimeSeconds: Math.floor(os.uptime()),
    loadAverage: os.loadavg(),
    memory: {
      totalBytes: os.totalmem(),
      freeBytes: os.freemem(),
      usedBytes: os.totalmem() - os.freemem(),
    },
    rootFilesystem: {
      filesystem,
      totalBytes: Number(totalBytes),
      usedBytes: Number(usedBytes),
      availableBytes: Number(availableBytes),
      usedPercent,
      mountPoint,
    },
  };
}

async function listContainers() {
  const { stdout } = await run("docker", ["ps", "-a", "--format", "{{json .}}"]);
  return {
    containers: stdout
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as Record<string, string>),
  };
}

async function containerLogs(name: string, lines: number) {
  const { stdout, stderr } = await run("docker", ["logs", "--tail", String(lines), name], 20_000);
  return {
    container: name,
    lines,
    logs: [stdout, stderr].filter(Boolean).join("\n").slice(-2_000_000),
    truncatedToBytes: 2_000_000,
  };
}

async function restartContainer(name: string) {
  const { stdout } = await run("docker", ["restart", "--time", "10", name], 30_000);
  return {
    container: name,
    restarted: true,
    result: stdout.trim(),
  };
}

async function failedServices() {
  const { stdout } = await run("systemctl", ["--failed", "--no-legend", "--no-pager", "--plain"]);
  return {
    failed: stdout
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean),
  };
}

async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://localhost");

  if (url.pathname === "/healthz" && req.method === "GET") {
    json(res, 200, { status: "ok", service: "infra-mcp-host-agent" });
    return;
  }

  if (!isAuthorized(req)) {
    json(res, 401, { error: "Unauthorized" });
    return;
  }

  if (req.method === "GET" && url.pathname === "/v1/system/status") {
    json(res, 200, await systemStatus());
    return;
  }

  if (req.method === "GET" && url.pathname === "/v1/docker/containers") {
    json(res, 200, await listContainers());
    return;
  }

  if (req.method === "GET" && url.pathname === "/v1/system/failed-services") {
    json(res, 200, await failedServices());
    return;
  }

  if (req.method === "POST" && url.pathname === "/v1/docker/logs") {
    const body = await readJsonBody(req);
    const name = requireContainerName(body.name);
    const lines = requireLogLines(body.lines);
    json(res, 200, await containerLogs(name, lines));
    return;
  }

  if (req.method === "POST" && url.pathname === "/v1/docker/restart") {
    const body = await readJsonBody(req);
    const name = requireContainerName(body.name);
    json(res, 200, await restartContainer(name));
    return;
  }

  json(res, 404, { error: "Not found" });
}

async function prepareSocket(): Promise<void> {
  await mkdir(path.dirname(socketPath), { recursive: true, mode: 0o750 });

  try {
    const stats = await lstat(socketPath);
    if (!stats.isSocket()) {
      throw new Error(`${socketPath} exists and is not a Unix socket`);
    }
    await unlink(socketPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
}

await prepareSocket();

const server = http.createServer((req, res) => {
  void handle(req, res).catch((error: unknown) => {
    const message = error instanceof Error ? error.message : "Unknown error";
    console.error("Host agent request failed", error);
    if (!res.headersSent) json(res, 500, { error: message });
    else res.end();
  });
});

server.listen(socketPath, async () => {
  await chmod(socketPath, socketMode);
  if (socketGid !== undefined && Number.isInteger(socketGid)) {
    await chown(socketPath, process.getuid?.() ?? 0, socketGid);
  }
  console.log(`infra-mcp host agent listening on ${socketPath}`);
});

function shutdown(): void {
  server.close(() => {
    void unlink(socketPath).catch(() => undefined);
    process.exit(0);
  });
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
