import type { Request, Response } from "express";
import { createMcpExpressApp } from "@modelcontextprotocol/sdk/server/express.js";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";

import { AgentClient } from "./agent-client.js";

const port = Number.parseInt(process.env.PORT ?? "3000", 10);
const bindHost = process.env.BIND_HOST ?? "127.0.0.1";
const socketPath = process.env.AGENT_SOCKET ?? "/run/infra-mcp/agent.sock";
const agentToken = process.env.AGENT_TOKEN ?? "";
const agent = new AgentClient(socketPath, agentToken);

const containerNameSchema = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9][A-Za-z0-9_.-]*$/, "Invalid container name");

function asToolResult(value: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(value, null, 2) }],
  };
}

function asToolError(error: unknown) {
  const message = error instanceof Error ? error.message : "Unknown error";
  return {
    isError: true,
    content: [{ type: "text" as const, text: message }],
  };
}

function createServer(): McpServer {
  const server = new McpServer(
    {
      name: "infra-mcp",
      version: "0.1.0",
    },
    {
      instructions:
        "Use read-only diagnostic tools before taking actions. Never invent container or service names. Restarts are consequential: inspect current status/logs first when troubleshooting, and only restart when the user explicitly asks or approves the action.",
    },
  );

  server.registerTool(
    "system_status",
    {
      title: "System status",
      description: "Read host uptime, CPU load, memory usage, and root filesystem usage.",
      inputSchema: {},
      annotations: {
        readOnlyHint: true,
        destructiveHint: false,
        openWorldHint: false,
      },
    },
    async () => {
      try {
        return asToolResult(await agent.get("/v1/system/status"));
      } catch (error) {
        return asToolError(error);
      }
    },
  );

  server.registerTool(
    "list_containers",
    {
      title: "List Docker containers",
      description: "List Docker containers on the host, including stopped containers and their current state.",
      inputSchema: {},
      annotations: {
        readOnlyHint: true,
        destructiveHint: false,
        openWorldHint: false,
      },
    },
    async () => {
      try {
        return asToolResult(await agent.get("/v1/docker/containers"));
      } catch (error) {
        return asToolError(error);
      }
    },
  );

  server.registerTool(
    "container_logs",
    {
      title: "Read container logs",
      description: "Read a bounded tail of logs from a named Docker container. Use this for live troubleshooting.",
      inputSchema: {
        name: containerNameSchema.describe("Exact Docker container name"),
        lines: z.number().int().min(1).max(1000).default(200).describe("Number of log lines to return"),
      },
      annotations: {
        readOnlyHint: true,
        destructiveHint: false,
        openWorldHint: false,
      },
    },
    async ({ name, lines }) => {
      try {
        return asToolResult(await agent.post("/v1/docker/logs", { name, lines }));
      } catch (error) {
        return asToolError(error);
      }
    },
  );

  server.registerTool(
    "failed_services",
    {
      title: "Failed system services",
      description: "List failed systemd services on the host for diagnostics.",
      inputSchema: {},
      annotations: {
        readOnlyHint: true,
        destructiveHint: false,
        openWorldHint: false,
      },
    },
    async () => {
      try {
        return asToolResult(await agent.get("/v1/system/failed-services"));
      } catch (error) {
        return asToolError(error);
      }
    },
  );

  server.registerTool(
    "restart_container",
    {
      title: "Restart Docker container",
      description: "Restart one existing named Docker container. This causes temporary service interruption.",
      inputSchema: {
        name: containerNameSchema.describe("Exact Docker container name"),
      },
      annotations: {
        readOnlyHint: false,
        destructiveHint: true,
        idempotentHint: true,
        openWorldHint: false,
      },
    },
    async ({ name }) => {
      try {
        return asToolResult(await agent.post("/v1/docker/restart", { name }));
      } catch (error) {
        return asToolError(error);
      }
    },
  );

  return server;
}

const app = createMcpExpressApp({ host: bindHost });

app.get("/healthz", (_req: Request, res: Response) => {
  res.status(200).json({ status: "ok", service: "infra-mcp" });
});

app.post("/mcp", async (req: Request, res: Response) => {
  const server = createServer();
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined,
  });

  try {
    await server.connect(transport);
    await transport.handleRequest(req, res, req.body);
  } catch (error) {
    console.error("MCP request failed", error);
    if (!res.headersSent) {
      res.status(500).json({
        jsonrpc: "2.0",
        error: { code: -32603, message: "Internal server error" },
        id: null,
      });
    }
  } finally {
    res.on("close", () => {
      void transport.close();
      void server.close();
    });
  }
});

app.get("/mcp", (_req: Request, res: Response) => {
  res.status(405).json({ error: "Method not allowed" });
});

app.delete("/mcp", (_req: Request, res: Response) => {
  res.status(405).json({ error: "Method not allowed" });
});

app.listen(port, bindHost, () => {
  console.log(`infra-mcp listening on http://${bindHost}:${port}/mcp`);
});
