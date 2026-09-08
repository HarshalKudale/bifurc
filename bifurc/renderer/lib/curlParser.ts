// Shared cURL command parser

export const SKIP_CURL_HEADERS = new Set([
  "host", "proxy-connection", "connection", "content-length", "transfer-encoding",
]);

function tokenizeCurl(input: string): string[] {
  const str = input.replace(/\\\r?\n/g, " ");
  const tokens: string[] = [];
  let i = 0;
  while (i < str.length) {
    while (i < str.length && /[ \t]/.test(str[i])) i++;
    if (i >= str.length) break;
    if (str[i] === "'") {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== "'") tok += str[i++];
      i++;
      tokens.push(tok);
    } else if (str[i] === '"') {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== '"') {
        if (str[i] === "\\" && i + 1 < str.length) { i++; tok += str[i]; }
        else tok += str[i];
        i++;
      }
      i++;
      tokens.push(tok);
    } else {
      let tok = "";
      while (i < str.length && !/[ \t]/.test(str[i])) tok += str[i++];
      tokens.push(tok);
    }
  }
  return tokens;
}

export function parseCurl(curlStr: string): {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
} {
  const tokens = tokenizeCurl(curlStr);
  let method = "";
  let url = "";
  const headers: Record<string, string> = {};
  let body = "";
  let i = 0;
  while (i < tokens.length) {
    const tok = tokens[i];
    if (tok === "curl") { i++; continue; }
    if (tok === "-X" || tok === "--request") {
      if (++i < tokens.length) method = tokens[i].toUpperCase();
    } else if (tok === "-H" || tok === "--header") {
      if (++i < tokens.length) {
        const ci = tokens[i].indexOf(":");
        if (ci > 0) headers[tokens[i].slice(0, ci).trim().toLowerCase()] = tokens[i].slice(ci + 1).trim();
      }
    } else if (tok === "-b" || tok === "--cookie") {
      if (++i < tokens.length) {
        headers["cookie"] = headers["cookie"] ? headers["cookie"] + "; " + tokens[i] : tokens[i];
      }
    } else if (tok === "-d" || tok === "--data" || tok === "--data-raw" || tok === "--data-binary" || tok === "--data-ascii") {
      if (++i < tokens.length) body = tokens[i];
    } else if (tok === "--url") {
      if (++i < tokens.length) url = tokens[i];
    } else if (!tok.startsWith("-") && !url && (tok.startsWith("http://") || tok.startsWith("https://"))) {
      url = tok;
    }
    i++;
  }
  if (!method) method = body ? "POST" : "GET";
  return { method, url, headers, body };
}
