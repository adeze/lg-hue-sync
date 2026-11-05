export function companionOrigin(value: string): string {
  const url = new URL(value.trim());
  if (
    !['http:', 'https:'].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    !['', '/'].includes(url.pathname)
  )
    throw new Error(
      'Enter an HTTP or HTTPS origin without credentials, a path, query or fragment.',
    );
  return url.origin;
}
