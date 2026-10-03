import { Component, type ReactNode } from 'react';
import { zh } from '../locales/zh-CN';
export class ErrorBoundary extends Component<{children: ReactNode}, {failed: boolean}> {
  state = {failed:false};
  static getDerivedStateFromError() { return {failed:true}; }
  render() { return this.state.failed ? <main role="alert">{zh.error.fallback}</main> : this.props.children; }
}
