import { Buffer } from "node:buffer";
import { spawn, exec } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Transform } from "node:stream";
import { text } from "node:stream/consumers";

const HTTP_PORT = 0; // 0 - random free one
const HEADER_SEPARATOR = Buffer.from("\r\n", "ascii");
const CONTENT_SEPARATOR = Buffer.from("\r\n\r\n", "ascii");
const NAME_VALUE_SEPARATOR = Buffer.from(": ", "ascii");
const LENGTH_HEADER = "Content-Length";
const TIMEOUT = 5_000;

const workdir = process.argv[1];
const bin = process.argv[2];
const args = process.argv.slice(3);

const PROXY_ID = Buffer.from(process.cwd().replace(/\/+$/, "")).toString("hex");
const PROXY_HTTP_PORT_FILE = join(workdir, "proxy", PROXY_ID);
const isWindows = process.platform === "win32";

const lsp = spawn(bin, args, {
  shell: (isWindows && bin.includes(".bat")) ? true : false,
  detached: false,
});

function cleanup() {
  if (!lsp || lsp.killed || lsp.exitCode !== null) {
    return;
  }

  if (isWindows) {
    // Windows: Use taskkill to kill the process tree (cmd.exe + the child)
    // /T = Tree kill (child processes), /F = Force
    exec(`taskkill /pid ${lsp.pid} /T /F`);
  } else {
    lsp.kill("SIGTERM");
    setTimeout(() => {
      if (!lsp.killed && lsp.exitCode === null) {
        lsp.kill("SIGKILL");
      }
    }, 1000);
  }
}

// Handle graceful IDE shutdown via stdin close
process.stdin.on("end", () => {
  cleanup();
  process.exit(0);
});
// Ensure node is monitoring the pipe
process.stdin.resume();

// Fallback: monitor parent process for ungraceful shutdown
setInterval(() => {
  try {
    // Check if parent is still alive
    process.kill(process.ppid, 0);
  } catch (e) {
    // On Windows, checking a process you don't own might throw EPERM.
    // We only want to kill if the error is ESRCH (No Such Process).
    if (e.code === "ESRCH") {
      cleanup();
      process.exit(0);
    }
    // If e.code is EPERM, the parent is alive but we don't have permission to signal it.
    // Do nothing.
  }
}, 5000);

const proxy = createLspProxy({ server: lsp, proxy: process });

proxy.on("client", (data, passthrough) => {
  passthrough();
});
proxy.on("server", (data, passthrough) => {
  passthrough();
});

const server = createServer(async (req, res) => {
  if (req.method !== "POST") {
    res.status = 405;
    res.end("Method not allowed");
    return;
  }

  const data = await text(req)
    .then(safeJsonParse)
    .catch(() => null);

  if (!data) {
    res.status = 400;
    res.end("Bad Request");
    return;
  }

  const result = await proxy.request(data.method, data.params);
  res.statusCode = 200;
  res.setHeader("Content-Type", "application/json");
  res.write(JSON.stringify(result));
  res.end();
}).listen(HTTP_PORT, () => {
  mkdirSync(dirname(PROXY_HTTP_PORT_FILE), { recursive: true });
  writeFileSync(PROXY_HTTP_PORT_FILE, server.address().port.toString());
});

export function createLspProxy({
  server: { stdin: serverStdin, stdout: serverStdout, stderr: serverStderr },
  proxy: { stdin: proxyStdin, stdout: proxyStdout, stderr: proxyStderr },
}) {
  const events = new EventEmitter();
  const queue = new Map();
  const nextid = iterid();

  proxyStdin.pipe(lspMessageSeparator()).on("data", (data) => {
    events.emit("client", parse(data), () => serverStdin.write(data));
  });

  serverStdout.pipe(lspMessageSeparator()).on("data", (data) => {
    const message = parse(data);

    const pending = queue.get(message?.id);
    if (pending) {
      pending(message);
      queue.delete(message.id);
      return;
    }

    events.emit("server", message, () => proxyStdout.write(data));
  });

  serverStderr.pipe(proxyStderr);

  return Object.assign(events, {
    /**
     *
     * @param {string} method
     * @param {any} params
     * @returns void
     */
    notification(method, params) {
      proxyStdout.write(stringify({ jsonrpc: "2.0", method, params }));
    },

    /**
     *
     * @param {string} method
     * @param {any} params
     * @returns Promise<any>
     */
    request(method, params) {
      return new Promise((resolve, reject) => {
        const id = nextid();
        queue.set(id, resolve);

        setTimeout(() => {
          if (queue.has(id)) {
            reject({
              jsonrpc: "2.0",
              id,
              error: {
                code: -32803,
                message: "Request to language server timed out after 5000ms.",
              },
            });
            this.cancel(id);
          }
        }, TIMEOUT);

        serverStdin.write(stringify({ jsonrpc: "2.0", id, method, params }));
      });
    },

    cancel(id) {
      queue.delete(id);

      serverStdin.write(
        stringify({
          jsonrpc: "2.0",
          method: "$/cancelRequest",
          params: { id },
        }),
      );
    },
  });
}

function iterid() {
  let acc = 1;
  return () => PROXY_ID + "-" + acc++;
}

/**
 * The base protocol consists of a header and a content part (comparable to HTTP).
 * The header and content part are separated by a ‘\r\n’.
 *
 * The header part consists of header fields.
 * Each header field is comprised of a name and a value,
 * separated by ‘: ‘ (a colon and a space).
 * The structure of header fields conforms to the HTTP semantic.
 * Each header field is terminated by ‘\r\n’.
 * Considering the last header field and the overall header
 * itself are each terminated with ‘\r\n’,
 * and that at least one header is mandatory,
 * this means that two ‘\r\n’ sequences always immediately precede
 * the content part of a message.
 *
 * @returns {Transform}
 * @see [language-server-protocol](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#headerPart)
 */
function lspMessageSeparator() {
  let buffer = Buffer.alloc(0);
  let contentLength = null;
  let headersLength = null;

  return new Transform({
    transform(chunk, encoding, callback) {
      buffer = Buffer.concat([buffer, chunk]);

      // A single chunk may contain multiple messages
      while (true) {
        // Wait until we get the whole headers block
        if (buffer.indexOf(CONTENT_SEPARATOR) === -1) {
          break;
        }

        if (!headersLength) {
          const headersEnd = buffer.indexOf(CONTENT_SEPARATOR);
          const headers = Object.fromEntries(
            buffer
              .subarray(0, headersEnd)
              .toString()
              .split(HEADER_SEPARATOR)
              .map((header) => header.split(NAME_VALUE_SEPARATOR))
              .map(([name, value]) => [name.toLowerCase(), value]),
          );

          // A "Content-Length" header must always be present
          contentLength = parseInt(headers[LENGTH_HEADER.toLowerCase()], 10);
          headersLength = headersEnd + CONTENT_SEPARATOR.length;
        }

        const msgLength = headersLength + contentLength;

        // Wait until we get the whole content part
        if (buffer.length < msgLength) {
          break;
        }

        this.push(buffer.subarray(0, msgLength));

        buffer = buffer.subarray(msgLength);
        contentLength = null;
        headersLength = null;
      }

      callback();
    },
  });
}

/**
 *
 * @param {any} content
 * @returns {string}
 */
function stringify(content) {
  const json = JSON.stringify(content);
  return (
    LENGTH_HEADER +
    NAME_VALUE_SEPARATOR +
    json.length +
    CONTENT_SEPARATOR +
    json
  );
}

/**
 *
 * @param {string} message
 * @returns {any | null}
 */
function parse(message) {
  const content = message.slice(message.indexOf(CONTENT_SEPARATOR));
  return safeJsonParse(content);
}

/**
 *
 * @param {string} json
 * @returns {any | null}
 */
function safeJsonParse(json) {
  try {
    return JSON.parse(json);
  } catch (err) {
    return null;
  }
}
