import { useState, type FormEvent } from 'react';
import { useCreateList } from '../api/queries';
import type { ListKind } from '../api/types';
import { LIST_KIND_LABEL, validateListName, validateListSource } from '../lib/lists';
import { Notice } from './States';

interface Props {
  kind: ListKind;
  onAdded: (text: string) => void;
}

/** Admin form that adds a list of the kind selected in the tab. */
export function ListForm({ kind, onAdded }: Props) {
  const create = useCreateList();
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [errors, setErrors] = useState<{ name?: string; url?: string }>({});

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const nameError = validateListName(name);
    const urlError = validateListSource(url);
    if (nameError || urlError) {
      setErrors({ name: nameError ?? undefined, url: urlError ?? undefined });
      return;
    }
    setErrors({});
    create.mutate(
      { kind, name: name.trim(), url: url.trim(), enabled },
      {
        onSuccess: (list) => {
          setName('');
          setUrl('');
          setEnabled(true);
          onAdded(`Added ${list.name} to ${LIST_KIND_LABEL[kind].toLowerCase()}.`);
        },
      },
    );
  };

  return (
    <form className="panel list-form" onSubmit={onSubmit} noValidate aria-labelledby="list-form-title">
      <div className="panel-head">
        <h2 id="list-form-title">Add to {LIST_KIND_LABEL[kind].toLowerCase()}</h2>
      </div>

      <div className="form-grid">
        <label className="field">
          <span>Name</span>
          <input
            name="name"
            value={name}
            placeholder="e.g. Ads and trackers"
            autoComplete="off"
            aria-invalid={errors.name ? true : undefined}
            aria-describedby={errors.name ? 'add-list-name-error' : undefined}
            onChange={(e) => {
              setName(e.target.value);
              if (errors.name) setErrors((prev) => ({ ...prev, name: undefined }));
            }}
          />
          {errors.name && <span id="add-list-name-error" className="field-error">{errors.name}</span>}
        </label>

        <label className="field wide">
          <span>URL or file path</span>
          <input
            name="url"
            inputMode="url"
            value={url}
            placeholder="https://example.com/hosts.txt or /etc/dnsui/lists/custom.txt"
            autoComplete="off"
            spellCheck={false}
            aria-invalid={errors.url ? true : undefined}
            aria-describedby={errors.url ? 'add-list-url-error' : undefined}
            onChange={(e) => {
              setUrl(e.target.value);
              if (errors.url) setErrors((prev) => ({ ...prev, url: undefined }));
            }}
          />
          {errors.url && <span id="add-list-url-error" className="field-error">{errors.url}</span>}
        </label>

        <label className="checkbox">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          <span>Enabled</span>
        </label>
      </div>

      {create.error && <Notice tone="error">{create.error.message}</Notice>}

      <div className="btn-row">
        <button type="submit" className="btn btn-primary" disabled={create.isPending}>
          {create.isPending ? 'Adding...' : 'Add list'}
        </button>
      </div>
    </form>
  );
}
