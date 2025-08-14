import { EventEmitter } from "node:events";
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { Transform } from "node:stream";
import { Buffer } from "node:buffer";

const HEADER_SEPARATOR = Buffer.from("\r\n", "ascii");
const CONTENT_SEPARATOR = Buffer.from("\r\n\r\n", "ascii");
const NAME_VALUE_SEPARATOR = Buffer.from(": ", "ascii");
const CONTENT_LENGTH = "Content-Length";

const bin = process.argv[1];
const args = process.argv.slice(2);

const jdtls = spawn(bin, args);

const proxy = createLspProxy({ server: jdtls, proxy: process });

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
    proxy.show(3, "Debug session running on port " + res.result);
    writeFileSync("./port.txt", res.result.toString());
  });

export function createLspProxy({
  server: { stdin: serverStdin, stdout: serverStdout, stderr: serverStderr },
  proxy: { stdin: proxyStdin, stdout: proxyStdout, stderr: proxyStderr },
}) {
  const events = new EventEmitter();
  const queue = new Map();
  const nextid = iterid();

  proxyStdin.pipe(lspMessageSeparator()).on("data", (data) => {
    events.emit("client", parse(data.toString()), () =>
      serverStdin.write(data),
    );
  });

  serverStdout.pipe(lspMessageSeparator()).on("data", (data) => {
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
     * @param {1 | 2 | 3 | 4 | 5} type
     * @param {string} message
     * @returns void
     */
    show(type, message) {
      proxyStdout.write(
        stringify({
          jsonrpc: "2.0",
          method: "window/showMessage",
          params: { type, message },
        }),
      );
    },

    /**
     *
     * @param {string} method
     * @param {any} params
     * @returns Promise<any>
     */
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
         * @see {https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#headerPart}
         */
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
          contentLength = parseInt(headers[CONTENT_LENGTH.toLowerCase()], 10);
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
    CONTENT_LENGTH +
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
  try {
    const content = message.slice(message.indexOf(CONTENT_SEPARATOR));
    return JSON.parse(content);
  } catch (err) {
    return null;
  }
}
