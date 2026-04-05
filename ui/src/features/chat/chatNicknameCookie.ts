const COOKIE_NAME = "tabularium_webui_chat_nick";
const MAX_AGE_SEC = 60 * 60 * 24 * 400;

function escapeCookieName(name: string): string {
  return name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function readChatNicknameFromCookie(): string | null {
  if (typeof document === "undefined") {
    return null;
  }
  const re = new RegExp(`(?:^|; )${escapeCookieName(COOKIE_NAME)}=([^;]*)`);
  const m = document.cookie.match(re);
  if (!m?.[1]) {
    return null;
  }
  try {
    const v = decodeURIComponent(m[1].trim());
    return v.length > 0 ? v : null;
  } catch {
    return null;
  }
}

export function writeChatNicknameCookie(nickname: string): void {
  const trimmed = nickname.trim();
  if (trimmed === "") {
    return;
  }
  const enc = encodeURIComponent(trimmed);
  document.cookie = `${COOKIE_NAME}=${enc}; Path=/; Max-Age=${MAX_AGE_SEC}; SameSite=Lax`;
}
