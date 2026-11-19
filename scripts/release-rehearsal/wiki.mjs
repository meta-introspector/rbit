// Repository wiki pages (new in 0.2.0) through their forms: a kept page, a renamed one and
// a deleted one, and the probe that finds each where it should be, or nowhere.
import { attributeAfter, expect } from "./http.mjs";

const pageId = (html) => html.match(/\/wiki\/-\/page\/(\d+)\/edit/)?.[1];

async function create(client, wiki, title, content) {
  const created = await client.form(`${wiki}/-/new`, { title, path: "", content, intent: "save" });
  expect(created.status === 303, `wiki page "${title}" is created`, created);
  const page = await client.get(created.location);
  const id = pageId(page.text);
  expect(id, `wiki page "${title}" offers an edit link`, page);
  return { id, url: created.location };
}

/** Turns the wiki of `repo` on and writes a kept, a renamed and a deleted page. */
export async function seedWiki(client, repo) {
  const wiki = `/${repo}/wiki`;
  const enabled = await client.form(`${wiki}/-/settings`, { enabled: "on" });
  expect(enabled.status === 303, "the wiki is turned on (wikis start off)", enabled);
  const kept = await create(client, wiki, "Upgrade notes", "Written after the upgrade.");

  const original = await create(client, wiki, "Old name", "This page is renamed.");
  const form = await client.get(`${wiki}/-/page/${original.id}/edit`);
  const base = attributeAfter(form.text, 'name="base"', "value");
  expect(base, "the edit form carries its base revision", form);
  const renamed = await client.form(`${wiki}/-/page/${original.id}/edit`, { title: "New name", path: "", content: "This page was renamed.", base, intent: "save" });
  expect(renamed.status === 303, "the wiki page is renamed", renamed);

  const doomed = await create(client, wiki, "Doomed page", "This page is deleted.");
  const confirm = await client.get(`${wiki}/-/page/${doomed.id}/delete`);
  const deleteBase = attributeAfter(confirm.text, 'name="base"', "value");
  expect(deleteBase, "the delete confirmation carries its base revision", confirm);
  const deleted = await client.form(`${wiki}/-/page/${doomed.id}/delete`, { base: deleteBase });
  expect(deleted.status === 303, "the wiki page is deleted", deleted);

  return { kept, renamedFrom: original.url, renamedTo: { id: original.id, url: renamed.location }, deleted: doomed.url };
}

/**
 * Probes the public repository's wiki as an anonymous reader. An administrator
 * sees a successful response with an enable form when a wiki is turned off.
 */
export async function probeWiki(checks, client, wiki, present) {
  const reader = client.fresh();
  await checks.check(`wiki pages written after the upgrade are ${present ? "present" : "absent"}`, async () => {
    const kept = await reader.get(wiki.kept.url);
    const renamed = await reader.get(wiki.renamedTo.url);
    for (const [page, response] of [[wiki.kept, kept], [wiki.renamedTo, renamed]]) {
      const found = response.status === 200 && response.text.includes(`/wiki/-/page/${page.id}/history"`);
      expect(present ? found : response.status === 404, `${page.url} is ${present ? "the seeded page" : "missing"}`, response);
    }
    return `kept ${kept.status}, renamed ${renamed.status}`;
  });
  await checks.check(present ? "old wiki name redirects and the deleted page stays deleted" : "old and deleted wiki paths are absent", async () => {
    const old = await reader.get(wiki.renamedFrom);
    const deleted = await reader.get(wiki.deleted);
    expect(present ? old.status === 308 && old.location === wiki.renamedTo.url : old.status === 404,
      present ? "the old wiki name redirects to the renamed page" : "the old wiki name is missing", old);
    expect(deleted.status === 404, "the deleted wiki page is missing", deleted);
    return `old name ${old.status}, deleted page ${deleted.status}`;
  });
}
