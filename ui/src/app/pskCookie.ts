/** Stage-1 operator convenience — not a vault; Ferrum may audit later. */

export const PSK_SESSION_STORAGE_KEY = "tabularium_x_auth_key";

export const PSK_TRUST_COOKIE_NAME = "tabularium_psk_trust";

const TRUST_MAX_AGE_SEC = 60 * 60 * 24 * 400;

export function readTrustedPskCookie(): string | null {
  const raw = `; ${document.cookie}`;
  const part = raw.split(`; ${PSK_TRUST_COOKIE_NAME}=`);
  if (part.length < 2) return null;
  const value = part.pop()?.split(";").shift();
  if (value === undefined || value === "") return null;
  try {
    const decoded = decodeURIComponent(value);
    return decoded.trim() === "" ? null : decoded;
  } catch {
    return null;
  }
}

export function writeTrustedPskCookie(psk: string): void {
  const enc = encodeURIComponent(psk);
  document.cookie = `${PSK_TRUST_COOKIE_NAME}=${enc};path=/;max-age=${TRUST_MAX_AGE_SEC};SameSite=Lax`;
}

export function clearTrustedPskCookie(): void {
  document.cookie = `${PSK_TRUST_COOKIE_NAME}=;path=/;max-age=0;SameSite=Lax`;
}
