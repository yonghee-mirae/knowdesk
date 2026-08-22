// `SearchHit.modifiedAt` is an RFC3339 UTC timestamp with nanosecond
// precision, as stored in `paths.modified_at`
// (`core/src/index/pipeline.rs`'s `format_system_time`). This renders it in
// the viewer's local time zone, truncated to millisecond precision.

export function formatLocalDateTime(rfc3339: string): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return rfc3339; // Unparsable - show as-is rather than hide it.

  const pad = (n: number, len = 2): string => String(n).padStart(len, '0');
  const y = date.getFullYear();
  const mo = pad(date.getMonth() + 1);
  const d = pad(date.getDate());
  const h = pad(date.getHours());
  const mi = pad(date.getMinutes());
  const s = pad(date.getSeconds());
  const ms = pad(date.getMilliseconds(), 3);
  return `${y}-${mo}-${d} ${h}:${mi}:${s}.${ms}`;
}
