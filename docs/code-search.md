# Code search

Open **Code search** in a repository to search literal, case-sensitive text on its
configured default branch. Filter by path prefix (`src/`) or detected language
(`Rust`, `Python`, `JavaScript`). Each matching line links to the searched commit,
so subsequent pushes do not change the destination. Existing read permissions
apply to both browser and API access. Search also works in archived repositories.

`GET /api/v1/repos/{owner}/{repo}/search/code?q=needle&path=src%2F&language=Rust`
returns `commit`, `matches` (`path`, `line`, `snippet`, `html_url`), `truncated`,
and `skipped_files`. The query is required and capped at 256 bytes. Search does not
interpret regular expressions. Binary files, non-UTF-8 files, submodules and
symlinks are not searched; text blobs above 1 MiB are skipped. Snippets are at
most 400 characters and begin near the match. Empty repositories return no matches.

Search reads Git directly and needs no index, migration or external service.
Two workers may run at once; additional requests receive 503 and may retry.
Each search stops after 100 matching lines, 20,000 tree entries, 32 MiB of object
contents, or a two-second cooperative deadline. Large tree objects are refused.
A partial result explicitly sets `truncated`; narrow filters when this appears.
Worker admission remains held if the client disconnects. This first version
searches one repository; cross-repository indexing is not implemented.
