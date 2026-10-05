/** Convert measured RMS to a readable level without moving the silence floor. */
export function waveformLevel(rms: number) {
  if (!Number.isFinite(rms) || rms <= .001) return 0;
  return Math.max(0, Math.min(1, (20 * Math.log10(rms) + 60) / 45));
}
export function waveformSample(previous: number, rms: number) {
  const target = waveformLevel(rms);
  return previous + (target - previous) * (target > previous ? .85 : .3);
}
