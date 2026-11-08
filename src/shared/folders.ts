/**
 * One spelling per folder, for looking a folder up: `C:\Code\App`, `c:/code/app/` and
 * `C:\code\app\` are the same folder on Windows. Works on text alone, without the file system.
 */
export function folderKey(folder: string, platform: string): string {
  const windows = platform === 'win32'
  let key = windows ? folder.replace(/[\\/]+/g, '\\') : folder.replace(/\/+/g, '/')
  // "C:\" and "/" keep their separator
  const root = windows ? /^[A-Za-z]:\\$/.test(key) : key === '/'
  if (!root) key = key.replace(/[\\/]$/, '')
  return windows ? key.toLowerCase() : key
}

/**
 * The account a folder is pinned to, if that account still exists. `accounts` maps folder keys
 * to profile ids.
 */
export function pinnedAccount(
  accounts: Readonly<Record<string, string>>,
  folder: string,
  platform: string,
  profiles: readonly { id: string }[]
): string | null {
  const id = accounts[folderKey(folder, platform)]
  return id !== undefined && profiles.some((p) => p.id === id) ? id : null
}
