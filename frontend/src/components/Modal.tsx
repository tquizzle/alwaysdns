import { useEffect, useId, type ReactNode } from 'react';

interface Props {
  title: string;
  onClose: () => void;
  children: ReactNode;
}

/** Centred dialog with a backdrop. Closes on Escape, the Close button or a backdrop click. */
export function Modal({ title, onClose, children }: Props) {
  const titleId = useId();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal panel" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <header className="panel-head">
          <h2 id={titleId}>{title}</h2>
          <button type="button" className="btn btn-small" onClick={onClose}>
            Close
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}
