import { useState, type FormEvent } from 'react';
import { useUpdateList } from '../api/queries';
import type { FilterList } from '../api/types';
import { validateListName, validateListUrl } from '../lib/lists';
import { Modal } from './Modal';
import { Notice } from './States';

interface Props {
  list: FilterList;
  onClose: () => void;
  onSaved: (text: string) => void;
}

/** Dialog that edits a list's name, URL and enabled state. */
export function ListDialog({ list, onClose, onSaved }: Props) {
  const update = useUpdateList();
  const [name, setName] = useState(list.name);
  const [url, setUrl] = useState(list.url);
  const [enabled, setEnabled] = useState(list.enabled);
  const [errors, setErrors] = useState<{ name?: string; url?: string }>({});

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const nameError = validateListName(name);
    const urlError = validateListUrl(url);
    if (nameError || urlError) {
      setErrors({ name: nameError ?? undefined, url: urlError ?? undefined });
      return;
    }
    setErrors({});
    const cleanName = name.trim();
    update.mutate(
      { id: list.id, body: { name: cleanName, url: url.trim(), enabled } },
      {
        onSuccess: () => {
          onSaved(`Saved ${cleanName}.`);
          onClose();
        },
      },
    );
  };

  return (
    <Modal title="Edit list" onClose={onClose}>
      <form className="dialog-form" onSubmit={onSubmit} noValidate>
        <label className="field">
          <span>Name</span>
          <input
            name="name"
            value={name}
            autoFocus
            aria-invalid={errors.name ? true : undefined}
            aria-describedby={errors.name ? 'edit-list-name-error' : undefined}
            onChange={(e) => {
              setName(e.target.value);
              if (errors.name) setErrors((prev) => ({ ...prev, name: undefined }));
            }}
          />
          {errors.name && <span id="edit-list-name-error" className="field-error">{errors.name}</span>}
        </label>

        <label className="field">
          <span>URL</span>
          <input
            name="url"
            type="url"
            inputMode="url"
            value={url}
            spellCheck={false}
            aria-invalid={errors.url ? true : undefined}
            aria-describedby={errors.url ? 'edit-list-url-error' : undefined}
            onChange={(e) => {
              setUrl(e.target.value);
              if (errors.url) setErrors((prev) => ({ ...prev, url: undefined }));
            }}
          />
          {errors.url && <span id="edit-list-url-error" className="field-error">{errors.url}</span>}
        </label>

        <label className="checkbox">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          <span>Enabled</span>
        </label>

        {update.error && <Notice tone="error">{update.error.message}</Notice>}

        <div className="btn-row end">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={update.isPending}>
            {update.isPending ? 'Saving...' : 'Save'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
