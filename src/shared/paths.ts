/**
 * A file path as it is typed into a terminal: bare when it is plain, quoted when it has spaces
 * or other characters a command line would split or interpret. Null for a path with control
 * characters, which must not reach a terminal as input.
 */
export function quotePath(path: string, platform: string): string | null {
  if (!path || Array.from(path).some((ch) => ch.charCodeAt(0) < 32 || ch.charCodeAt(0) === 127)) return null
  if (platform === 'win32') {
    // a Windows path cannot contain a double quote, so wrapping it in them is always safe
    return /^[A-Za-z0-9_\-.:\\/]+$/.test(path) ? path : `"${path}"`
  }
  return /^[A-Za-z0-9_\-./]+$/.test(path) ? path : `'${path.split("'").join("'\\''")}'`
}

/** The text that dropping these files on a terminal types: their paths, separated and followed by a space. */
export function pathsAsInput(paths: readonly string[], platform: string): string {
  const quoted = paths.map((p) => quotePath(p, platform)).filter((p): p is string => p !== null)
  return quoted.length > 0 ? `${quoted.join(' ')} ` : ''
}
