const SITE_URL = 'https://alwaysdns.dev';
const AUTHOR_URL = 'https://github.com/tquizzle';
const REPO_URL = 'https://github.com/tquizzle/alwaysdns';

/** Site footer, shown under every page and on the sign-in screen. */
export function Footer() {
  return (
    <footer className="site-footer">
      <p>
        Made with{' '}
        <span role="img" aria-label="love">
          ❤️
        </span>{' '}
        by{' '}
        <a href={AUTHOR_URL} target="_blank" rel="noopener noreferrer">
          TQ
        </a>
      </p>
      <nav aria-label="Project links">
        <a href={SITE_URL} target="_blank" rel="noopener noreferrer">
          alwaysdns.dev
        </a>
        <a href={REPO_URL} target="_blank" rel="noopener noreferrer">
          Source on GitHub
        </a>
      </nav>
    </footer>
  );
}
