/** Ask the Throne whether this key opens the gate (`GET /api/whoami`). */

export type VerifyPskResult =
  | { ok: true }
  | { ok: false; message: string };

export async function verifyPskWithServer(psk: string): Promise<VerifyPskResult> {
  const trimmed = psk.trim();
  if (trimmed === "") {
    return { ok: false, message: "Enter a pre-shared key." };
  }
  try {
    const r = await fetch("/api/whoami", {
      headers: { "X-Auth-Key": trimmed },
    });
    if (r.ok) {
      return { ok: true };
    }
    if (r.status === 401) {
      return {
        ok: false,
        message: "Incorrect or unknown pre-shared key.",
      };
    }
    return {
      ok: false,
      message: `The forge refused this key (${r.status}).`,
    };
  } catch {
    return { ok: false, message: "Could not reach the server." };
  }
}
