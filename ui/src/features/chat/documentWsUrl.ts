/** Build same-origin `/ws` URL; `wss:` when the page is HTTPS (browser cannot do CLI-style headers). */
export function documentWsUrl(): string {
  const { protocol, host } = window.location;
  const wsProto = protocol === "https:" ? "wss:" : "ws:";
  return `${wsProto}//${host}/ws`;
}
