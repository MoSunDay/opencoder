import { Component, type ErrorInfo, type ReactNode } from "react";

export default class SceneBoundary extends Component<{ children: ReactNode; onError: (message: string) => void }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error, _info: ErrorInfo) { this.props.onError(error.message || "浏览器无法启动三维画布"); }
  render() { return this.state.failed ? null : this.props.children; }
}
