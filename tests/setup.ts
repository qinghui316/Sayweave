import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';
afterEach(cleanup);

// jsdom has no layout scrolling; real focus/scroll geometry is covered by browser tests.
HTMLElement.prototype.scrollIntoView ??= function () {};
