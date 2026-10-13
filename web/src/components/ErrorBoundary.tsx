/**
 * 渲染期异常兜底：任何子组件在渲染 / 生命周期中抛错时显示可读面板，而不是整页白屏。
 *
 * 注意：ErrorBoundary 只捕获渲染期错误；事件处理器与异步请求的错误仍需调用点
 * 各自 try/catch（本项目统一走 toast）。
 */
import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Banner, Button } from './ui';

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  override state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    // 控制台保留完整堆栈便于定位，页面只展示简短信息。
    console.error('unhandled render error', error, info);
  }

  private readonly reset = (): void => this.setState({ error: null });

  override render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <>
        <Banner tone="error">
          <div>页面渲染出错，已阻止白屏。</div>
          <div className="small mt-8 mono break-all">{error.message}</div>
        </Banner>
        <div className="row mt-8">
          <Button onClick={this.reset}>重试渲染</Button>
        </div>
      </>
    );
  }
}
