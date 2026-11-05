interface Props {
  /** The question shown beside the buttons, for example "Delete this list?". */
  prompt: string;
  /** True while this row's delete request is running. */
  deleting: boolean;
  /** True while any delete is running, so a second one cannot start. */
  disabled?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Second step of a delete. Focus starts on Cancel so a stray Enter never deletes. */
export function ConfirmDelete({ prompt, deleting, disabled = false, onConfirm, onCancel }: Props) {
  return (
    <div className="row-actions" role="group" aria-label={prompt}>
      <span className="small text-red">{prompt}</span>
      <button type="button" className="btn btn-small btn-danger" disabled={disabled || deleting} onClick={onConfirm}>
        {deleting ? 'Deleting...' : 'Confirm'}
      </button>
      <button type="button" className="btn btn-small" autoFocus onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}
