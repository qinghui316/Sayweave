import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { PasswordInput } from './controls';
import { CopyFeedbackButton } from './CopyFeedbackButton';
import { AsyncStatusFeedback } from './AsyncStatusFeedback';
import { VoiceControl } from './VoiceControl';
import { AnimatedIcon } from './AnimatedIcon';
import { MotionScope } from './useMotion';
import { LoadingIndicator } from './controls';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); Object.defineProperty(Element.prototype, 'animate', { configurable: true, value: undefined }); });
function deferred() { let resolve!: () => void; let reject!: () => void; const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }

test('real speech feedback uses recognition labels independently of microphone levels', () => {
  const actions = { onStart: vi.fn(), onFinish: vi.fn(), onCancel: vi.fn() };
  const { rerender } = render(<VoiceControl mode="speech" phase="failed" {...actions}/>);
  expect(screen.getByRole('group', { name: '识别失败' })).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: '重新开始录音' }));
  expect(actions.onStart).toHaveBeenCalledOnce();
  rerender(<VoiceControl mode="speech" phase="completed" {...actions}/>);
  expect(screen.getByRole('group', { name: '识别完成' })).toBeVisible();
  expect(screen.getByRole('status')).toHaveTextContent('识别完成');
  rerender(<VoiceControl mode="speech" phase="recording" {...actions}/>);
  expect(screen.getByRole('button', { name: '结束录音' })).toBeVisible();
});
test('password repeatedly toggles without losing selection, input node or button focus', () => {
  render(<PasswordInput aria-label="密钥" defaultValue="abcdef"/>);
  const input = screen.getByLabelText('密钥') as HTMLInputElement;
  input.setSelectionRange(2, 4, 'backward');
  const button = screen.getByRole('button', { name: '显示内容' });
  button.focus();
  for (let i = 0; i < 6; i++) { fireEvent.click(button); expect(input.value).toBe('abcdef'); expect(input.selectionStart).toBe(2); expect(input.selectionEnd).toBe(4); expect(button).toHaveFocus(); expect(screen.getByLabelText('密钥')).toBe(input); }
  expect(input.type).toBe('password');
});
test('copy awaits result, stays failed until retry, ignores older completion and expiry', async () => {
  vi.useFakeTimers();
  const old = deferred(), next = deferred(), retry = deferred();
  const onCopy = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise).mockReturnValueOnce(retry.promise);
  render(<CopyFeedbackButton onCopy={onCopy}/>);
  const button = screen.getByRole('button');
  fireEvent.click(button); expect(button).toHaveTextContent('复制中');
  fireEvent.click(button); await act(async () => next.reject()); expect(button).toHaveTextContent('复制失败，重试');
  await act(async () => old.resolve()); expect(button).toHaveTextContent('复制失败，重试');
  fireEvent.click(button); await act(async () => retry.resolve()); expect(button).toHaveTextContent('已复制');
  act(() => vi.advanceTimersByTime(1499)); expect(button).toHaveTextContent('已复制');
  act(() => vi.advanceTimersByTime(1)); expect(button).toHaveTextContent('复制');
});
test('new copy invalidates success timer; unmount prevents promise/timer updates', async () => {
  vi.useFakeTimers(); const next = deferred();
  const onCopy = vi.fn().mockResolvedValueOnce(undefined).mockReturnValueOnce(next.promise);
  const { unmount } = render(<CopyFeedbackButton onCopy={onCopy}/>);
  fireEvent.click(screen.getByRole('button')); await act(async () => {});
  fireEvent.click(screen.getByRole('button')); act(() => vi.advanceTimersByTime(1500));
  expect(screen.getByRole('button')).toHaveTextContent('复制中');
  unmount(); await act(async () => next.resolve()); expect(vi.getTimerCount()).toBe(0);
});
test('controlled async status does not infer completion and keeps SVG instance', () => {
  const { rerender, container } = render(<AsyncStatusFeedback status="pending" message="等待" motionEnabled={false}/>);
  const svg = container.querySelector('svg');
  expect(screen.getByRole('status')).toHaveTextContent('等待');
  rerender(<AsyncStatusFeedback status="success" message="已连接" motionEnabled={false}/>);
  expect(container.querySelector('svg')).toBe(svg);
  expect(screen.getByRole('status')).toHaveTextContent('已连接');
  rerender(<AsyncStatusFeedback status="error" message="重试" motionEnabled={false}/>);
  expect(screen.getByRole('status')).toHaveTextContent('重试');
});
test('voice reuses result SVG and suppresses hidden completion replay', () => {
  const animate = vi.fn(() => ({ cancel: vi.fn() }));
  Object.defineProperty(Element.prototype, 'animate', { configurable: true, value: animate });
  const actions = { onStart: vi.fn(), onFinish: vi.fn(), onCancel: vi.fn() };
  const { container, rerender } = render(<VoiceControl phase="processing" {...actions}/>);
  const svg = container.querySelector('[data-status-glyph] svg');
  rerender(<VoiceControl phase="completed" {...actions}/>); expect(animate).toHaveBeenCalledOnce();
  expect(container.querySelector('[data-status-glyph] svg')).toBe(svg);
  rerender(<VoiceControl phase="failed" {...actions} motionEnabled={false}/>);
  rerender(<VoiceControl phase="completed" {...actions} motionEnabled={false}/>);
  rerender(<VoiceControl phase="completed" {...actions} motionEnabled/>);
  expect(animate).toHaveBeenCalledOnce();
  rerender(<VoiceControl phase="recording" {...actions} motionEnabled={false}/>);
  expect(screen.getByRole('group')).toHaveAttribute('data-motion', 'static');
  fireEvent.click(screen.getByRole('button', { name: '取消演示' })); expect(actions.onCancel).toHaveBeenCalledOnce();
});
test('first/reduced/hidden icon uses immediate target with no spinning', () => {
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
  const { container, rerender } = render(<AnimatedIcon icon="pending" spinning/>);
  const svg = container.querySelector('svg'); expect(container.firstChild).toHaveAttribute('data-motion', 'static');
  rerender(<AnimatedIcon icon="check"/>); expect(container.querySelector('svg')).toBe(svg);
  expect(container.firstChild).toHaveAttribute('data-icon', 'check'); expect(svg?.getAttribute('class')).toBeNull();
  vi.unstubAllGlobals();
});
test('offscreen and document-hidden icons stop and restore at the latest target', () => {
  let observe!: (entries: { isIntersecting: boolean }[]) => void;
  const disconnect = vi.fn();
  vi.stubGlobal('IntersectionObserver', class { constructor(callback: typeof observe) { observe = callback; } observe() {} disconnect = disconnect; });
  const { container, rerender } = render(<AnimatedIcon icon="pending" spinning/>);
  act(() => observe([{ isIntersecting: false }])); expect(container.firstChild).toHaveAttribute('data-motion', 'static');
  rerender(<AnimatedIcon icon="check"/>); expect(container.firstChild).toHaveAttribute('data-icon', 'check');
  act(() => observe([{ isIntersecting: true }])); expect(container.firstChild).toHaveAttribute('data-motion', 'active');
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  fireEvent(document, new Event('visibilitychange')); expect(container.firstChild).toHaveAttribute('data-motion', 'static');
  hidden.mockReturnValue(false); fireEvent(document, new Event('visibilitychange')); expect(container.firstChild).toHaveAttribute('data-motion', 'active');
});
test('native window motion scope stops all nested feedback loops without changing their status', () => {
  const children = <MotionScope enabled><AsyncStatusFeedback status="pending" message="等待"/><LoadingIndicator/></MotionScope>;
  const { container, rerender } = render(<MotionScope enabled>{children}</MotionScope>);
  expect(container.querySelector('[data-icon]')).toHaveAttribute('data-motion', 'active');
  rerender(<MotionScope enabled={false}>{children}</MotionScope>);
  expect(container.querySelector('[data-icon]')).toHaveAttribute('data-motion', 'static');
  expect(container.querySelector('[style*="animation-play-state"]')).toHaveStyle({ animationPlayState: 'paused' });
  expect(screen.getByRole('status')).toHaveTextContent('等待');
});
