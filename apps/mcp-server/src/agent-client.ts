import http from "node:http";

export class AgentClient {
  constructor(
    private readonly socketPath: string,
    private readonly token: string,
  ) {
    if (!token || token.length < 32) {
      throw new Error("AGENT_TOKEN must be configured with at least 32 characters");
    }
  }

  get<T>(path: string): Promise<T> {
    return this.request<T>("GET", path);
  }

  post<T>(path: string, body: unknown): Promise<T> {
    return this.request<T>("POST", path, body);
  }

  private request<T>(method: "GET" | "POST", path: string, body?: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const payload = body === undefined ? undefined : JSON.stringify(body);
      const request = http.request(
        {
          socketPath: this.socketPath,
          path,
          method,
          headers: {
            authorization: `Bearer ${this.token}`,
            accept: "application/json",
            ...(payload
              ? {
                  "content-type": "application/json",
                  "content-length": Buffer.byteLength(payload),
                }
              : {}),
          },
        },
        (response) => {
          const chunks: Buffer[] = [];
          response.on("data", (chunk: Buffer) => chunks.push(chunk));
          response.on("end", () => {
            const raw = Buffer.concat(chunks).toString("utf8");
            const status = response.statusCode ?? 500;

            if (status < 200 || status >= 300) {
              reject(new Error(`Host agent returned ${status}: ${raw.slice(0, 2000)}`));
              return;
            }

            try {
              resolve(JSON.parse(raw) as T);
            } catch {
              reject(new Error("Host agent returned invalid JSON"));
            }
          });
        },
      );

      request.setTimeout(15_000, () => {
        request.destroy(new Error("Host agent request timed out"));
      });
      request.on("error", reject);
      if (payload) request.write(payload);
      request.end();
    });
  }
}
