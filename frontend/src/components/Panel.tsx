import type { ReactNode } from 'react';

interface Props {
  title: string;
  /** One line under the title, for what the numbers mean. */
  subtitle?: string;
  actions?: ReactNode;
  className?: string;
  children: ReactNode;
}

export function Panel({ title, subtitle, actions, className = '', children }: Props) {
  return (
    <section className={`panel ${className}`}>
      <header className="panel-head">
        <h2>{title}</h2>
        {actions}
      </header>
      {subtitle && <p className="panel-sub muted small">{subtitle}</p>}
      {children}
    </section>
  );
}
