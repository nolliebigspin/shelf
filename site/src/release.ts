export const repo = "nolliebigspin/shelf";
export const repoUrl = `https://github.com/${repo}`;

// The release workflow uploads the disk image a second time under this fixed name,
// so GitHub's /releases/latest/download/ link always serves the newest version
// without touching the site.
export const downloadUrl = `${repoUrl}/releases/latest/download/Shelf_universal.dmg`;

/**
 * Newest release version (e.g. "0.2.0"), read at build time from where
 * github.com/<repo>/releases/latest redirects to (no API rate limit).
 * Null when GitHub can't be reached; the page then just says "Latest version".
 */
export async function latestVersion(): Promise<string | null> {
  try {
    const res = await fetch(`${repoUrl}/releases/latest`, { method: "HEAD", redirect: "manual" });
    const tag = res.headers.get("location")?.match(/\/releases\/tag\/v?([^/?#]+)$/)?.[1];
    return tag ?? null;
  } catch {
    return null;
  }
}
