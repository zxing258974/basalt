// 命令行的纯逻辑:回复照 redis-cli 的样子排成一行行(带上颜色用的类别)、提示符、回显时遮住密码。
import { carriesSecret, commandVerb, mightCarrySecret, splitArgs } from "../../lib/redisArgs";
import type { Reply } from "../../lib/redisTypes";

export type Tone = "string" | "int" | "nil" | "error" | "status" | "meta";

export interface Segment {
  text: string;
  tone: Tone;
}

/** 一行输出:前面的编号(`1) `、`   2) `)不上色,后面是内容。 */
export interface OutputLine {
  prefix: string;
  segments: Segment[];
}

/** 这几个命令的字符串回复原样一行行显示(redis-cli 也是),不加引号转义。 */
const RAW = new Set(["INFO", "CLIENT LIST", "CLIENT INFO", "MEMORY DOCTOR", "LATENCY DOCTOR", "LOLWUT"]);

/** 字符串加双引号,引号、反斜杠、控制字符转义(和 redis-cli 一样;中文等原样)。 */
export function quoteText(text: string): string {
  let out = '"';
  for (const c of text) {
    if (c === '"') out += '\\"';
    else if (c === "\\") out += "\\\\";
    else if (c === "\n") out += "\\n";
    else if (c === "\r") out += "\\r";
    else if (c === "\t") out += "\\t";
    else {
      const code = c.codePointAt(0)!;
      out += code < 0x20 || code === 0x7f ? `\\x${code.toString(16).padStart(2, "0")}` : c;
    }
  }
  return `${out}"`;
}

const line = (text: string, tone: Tone): OutputLine => ({ prefix: "", segments: [{ text, tone }] });

/** `verb` 是命令名(见 commandVerb),决定字符串要不要原样显示。 */
export function formatReply(reply: Reply, verb = ""): OutputLine[] {
  if (reply.kind === "bulk" && RAW.has(verb) && !reply.binary) {
    const lines = reply.text.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
    return [...lines.map((text) => line(text, "string")), ...truncatedNote(reply)];
  }
  return format(reply);
}

function truncatedNote(reply: Extract<Reply, { kind: "bulk" }>): OutputLine[] {
  if (!reply.truncated) return [];
  const total = reply.bytes.toLocaleString();
  if (reply.binary) return [line(`（太长，只显示了前面一部分，一共 ${total} 字节）`, "meta")];
  const shown = new TextEncoder().encode(reply.text).length.toLocaleString();
  return [line(`（只显示了前 ${shown} 字节，一共 ${total} 字节）`, "meta")];
}

function format(reply: Reply): OutputLine[] {
  switch (reply.kind) {
    case "nil":
      return [line("(nil)", "nil")];
    case "status":
      return [line(reply.text, "status")];
    case "error":
      return [line(`(error) ${reply.text}`, "error")];
    case "int":
      return [line(`(integer) ${reply.value}`, "int")];
    case "number":
      return [line(`(double) ${reply.text}`, "int")];
    case "bool":
      return [line(reply.value ? "(true)" : "(false)", "int")];
    case "bulk":
      return [line(reply.binary ? `"${reply.text}"` : quoteText(reply.text), "string"), ...truncatedNote(reply)];
    case "array":
      if (reply.total === 0) return [line("(empty array)", "nil")];
      return numbered(
        reply.items.map((item) => format(item)),
        reply.total,
        ")",
      );
    case "map":
      if (reply.total === 0) return [line("(empty hash)", "nil")];
      return numbered(
        reply.entries.map(([key, value]) => {
          const [first, ...rest] = format(value);
          const head = format(key)[0]!;
          const joined: OutputLine = {
            prefix: "",
            segments: [...head.segments, { text: " => ", tone: "meta" }, ...(first?.segments ?? [])],
          };
          const pad = " ".repeat(head.segments.reduce((n, s) => n + s.text.length, 0) + 4);
          return [joined, ...rest.map((l) => ({ ...l, prefix: pad + l.prefix }))];
        }),
        reply.total,
        "#",
      );
  }
}

/**
 * 编号 `1) `,编号按最大的那个对齐;嵌套的值第一行跟在编号后面,其余行缩进到同一列。
 * 只带回了一部分时最后注明还有多少项。
 */
function numbered(items: OutputLine[][], total: number, mark: string): OutputLine[] {
  const width = String(items.length).length;
  const out = items.flatMap((lines, i) => {
    const label = `${String(i + 1).padStart(width)}${mark} `;
    const indent = " ".repeat(label.length);
    return lines.map((l, k) => ({ ...l, prefix: (k === 0 ? label : indent) + l.prefix }));
  });
  if (total > items.length) out.push(line(`…还有 ${(total - items.length).toLocaleString()} 项没显示`, "meta"));
  return out;
}

/** 提示符:`127.0.0.1:6379[2]>`,0 号库不写库号,事务里加 `(TX)`(和 redis-cli 一样)。 */
export function promptText(host: string, db: number, transaction: boolean): string {
  return `${host}${db === 0 ? "" : `[${db}]`}${transaction ? "(TX)" : ""}>`;
}

/** 回显用:带密码的命令只留命令名;拆不开的行看着像带密码的也遮住。 */
export function displayLine(line: string): string {
  const args = splitArgs(line);
  if ("error" in args) return mightCarrySecret(line) ? `${line.trim().split(/\s+/)[0]!.toUpperCase()} ••••••` : line;
  return carriesSecret(args) ? `${commandVerb(args)} ••••••` : line;
}

/** 输入框里的一段文字 → 要依次执行的命令(每行一条,空行跳过)。 */
export function commandLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "");
}

/** 一行命令的命令名(给格式化用);拆不了就是空的。 */
export function verbOf(line: string): string {
  const args = splitArgs(line);
  return "error" in args ? "" : commandVerb(args);
}
