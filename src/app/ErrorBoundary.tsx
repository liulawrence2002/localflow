import { Component, type ReactNode } from "react";

interface ErrorBoundaryProps {
  /** What failed, for the message: "This screen". */
  label: string;
  children: ReactNode;
}

interface ErrorBoundaryState {
  failed: boolean;
}

/**
 * Keeps a fault in one screen from blanking the whole settings window. The error is not shown
 * or logged with its details, which could include settings text.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { failed: false };

  // No componentDidCatch: nothing is logged, because the error could carry typed text.
  static getDerivedStateFromError(): ErrorBoundaryState {
    return { failed: true };
  }

  render() {
    if (!this.state.failed) {
      return this.props.children;
    }
    return (
      <div className="notice notice--error" role="alert">
        <div className="notice__body">
          <p className="notice__title">{this.props.label} could not be shown.</p>
          <p className="notice__text">
            Your settings are unchanged. Try again, or choose another section and come back.
          </p>
          <div className="button-row notice__actions">
            <button type="button" onClick={() => this.setState({ failed: false })}>
              Try again
            </button>
          </div>
        </div>
      </div>
    );
  }
}
