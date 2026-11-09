// A planner-side MCP client on one of the harness's in-process servers (createSdkMcpServer).

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";

export async function connect(server: { instance: { connect: (t: InMemoryTransport) => Promise<void> } }) {
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  await server.instance.connect(serverSide);
  const mcp = new Client({ name: "test", version: "0" });
  await mcp.connect(clientSide);
  return async (name: string, args: Record<string, unknown> = {}) => {
    const result = (await mcp.callTool({ name, arguments: args })) as CallToolResult;
    return { text: result.content.map((c) => (c.type === "text" ? c.text : "")).join(""), error: result.isError === true };
  };
}
