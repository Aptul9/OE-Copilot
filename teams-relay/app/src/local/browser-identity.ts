// Match the Windows identity verified with the BIP sign-in flow. Keep native identity on other platforms.
export function windowsBrowserArgs(platform: NodeJS.Platform = process.platform): string[] {
  return platform === "linux"
    ? ["--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36"]
    : [];
}
