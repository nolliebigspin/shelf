export const repo = "nolliebigspin/shelf";
export const repoUrl = `https://github.com/${repo}`;

// The release workflow uploads each installer a second time under a fixed name,
// so GitHub's /releases/latest/download/ links always serve the newest version
// without touching the site.
const latest = (file: string) => `${repoUrl}/releases/latest/download/${file}`;

export const downloads = {
  mac: { name: "Mac", url: latest("Shelf_universal.dmg"), note: "Apple Silicon and Intel" },
  windows: { name: "Windows", url: latest("Shelf_windows_x64-setup.exe"), note: "Windows 10 and 11" },
  linux: { name: "Linux", url: latest("Shelf_linux_amd64.AppImage"), note: "x64 AppImage" },
} as const;
export const debUrl = latest("Shelf_linux_amd64.deb");

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
