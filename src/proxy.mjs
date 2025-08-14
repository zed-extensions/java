import { EventEmitter } from "node:events";
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { Transform } from "node:stream";

const HEADER_SEPARATOR_BUFFER = Buffer.from("\r\n\r\n");
const CONTENT_LENGTH_PREFIX_BUFFER = Buffer.from("content-length: ");
const HEADER_SEPARATOR = "\r\n\r\n";
const CONTENT_LENGTH_PREFIX = "Content-Length: ";

const bin = process.argv[1];
const args = process.argv.slice(2);

const jdtls = spawn(bin, args);

const proxy = createJsonRpcProxy({ server: jdtls, proxy: process });

proxy.on("server", (data, passthrough) => {
  passthrough();
});

proxy.on("client", (data, passthrough) => {
  passthrough();
});

proxy
  .send("workspace/executeCommand", {
    command: "vscode.java.startDebugSession",
  })
  .then((res) => {
    writeFileSync("./port.txt", res.result.toString());
  });

export function createJsonRpcProxy({
  server: { stdin: serverStdin, stdout: serverStdout, stderr: serverStderr },
  proxy: { stdin: proxyStdin, stdout: proxyStdout, stderr: proxyStderr },
}) {
  const events = new EventEmitter();
  const queue = new Map();
  const nextid = iterid();

  proxyStdin.pipe(jsonRpcSeparator()).on("data", (data) => {
    events.emit("client", parse(data.toString()), () =>
      serverStdin.write(data),
    );
  });

  serverStdout.pipe(jsonRpcSeparator()).on("data", (data) => {
    const message = parse(data.toString());

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
     * @param {'error' | 'warning' | 'info' | 'log'} type
     * @param {string} message
     */
    log(type, message) {
      proxyStdout.write(
        JSON.stringify({
          jsonrpc: "2.0",
          method: "window/logMessage",
          params: { type, message },
        }),
      );
    },

    send(method, params) {
      return new Promise((resolve) => {
        const id = nextid();
        queue.set(id, resolve);

        serverStdin.write(stringify({ jsonrpc: "2.0", id, method, params }));
      });
    },
  });
}

function iterid() {
  let acc = 1;
  return () => "zed-java-proxy-" + acc++;
}

function jsonRpcSeparator() {
  let buffer = Buffer.alloc(0);
  let contentLength = null;

  return new Transform({
    transform(chunk, encoding, callback) {
      buffer = Buffer.concat([buffer, chunk]);

      while (true) {
        const headerEndIndex = buffer.indexOf(HEADER_SEPARATOR_BUFFER);
        if (headerEndIndex === -1) {
          break;
        }

        if (contentLength === null) {
          const headersBuffer = buffer.subarray(0, headerEndIndex);
          const headers = headersBuffer.toString("utf-8").toLowerCase();
          const lines = headers.split("\r\n");
          let newContentLength = 0;
          let foundLength = false;

          for (const line of lines) {
            if (line.startsWith(CONTENT_LENGTH_PREFIX_BUFFER.toString())) {
              const lengthString = line
                .substring(CONTENT_LENGTH_PREFIX_BUFFER.length)
                .trim();
              const parsedLength = parseInt(lengthString, 10);

              if (isNaN(parsedLength) || parsedLength < 0) {
                this.destroy(
                  new Error(`Invalid Content-Length header: '${lengthString}'`),
                );
                return;
              }

              newContentLength = parsedLength;
              foundLength = true;
              break;
            }
          }

          if (!foundLength) {
            this.destroy(new Error("Missing Content-Length header"));
            return;
          }

          contentLength = newContentLength;
        }

        const headerLength = headerEndIndex + HEADER_SEPARATOR_BUFFER.length;
        const totalMessageLength = headerLength + contentLength;

        if (buffer.length < totalMessageLength) {
          break;
        }

        const fullMessage = buffer.subarray(0, totalMessageLength);

        this.push(fullMessage);
        buffer = buffer.subarray(totalMessageLength);
        contentLength = null;
      }

      callback();
    },
  });
}

function stringify(request) {
  const json = JSON.stringify(request);
  return CONTENT_LENGTH_PREFIX + json.length + HEADER_SEPARATOR + json;
}

function parse(response) {
  try {
    return JSON.parse(response.split("\n").at(-1));
  } catch (err) {
    return null;
  }
}
