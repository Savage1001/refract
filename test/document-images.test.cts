const {test} = require('node:test');
import assert = require('node:assert/strict');
import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
const {ProjectStore} = require('../src/project-store.cjs');
const {ChatService} = require('../src/chat.cjs');
const {materializeDocumentImages} = require('../src/document-images.cjs');

async function fixture(t: import('node:test').TestContext) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-document-images-'));
  t.after(() => fs.rm(root, {recursive: true, force: true}));
  const store = new ProjectStore(root), project = await store.create('Paper');
  const bytes = Buffer.from([137,80,78,71,13,10,26,10,1]);
  const service = new ChatService(store);
  const used = await service.attachPastedImage(project.id, bytes);
  const reference = await service.attachPastedImage(project.id, bytes);
  return {store, project, bytes, used, reference};
}

test('only images included in LaTeX become visible, durable project assets', async t => {
  const {store, project, bytes, used, reference} = await fixture(t);
  const privatePath = `.refract-attachments/${used.token}`;
  const source = `\\usepackage{graphicx}\n\\includegraphics[width=.5\\textwidth]{${privatePath}}\n\\includegraphics*{\\detokenize{${privatePath}}}\n`;
  await store.write(project.id, 'chapters/figures.tex', source);
  await materializeDocumentImages(store, project.id);
  const asset = `images/chat-${used.token}`;
  assert.deepEqual(await fs.readFile(await store.existingFile(project.id, asset)), bytes);
  assert.equal(await store.read(project.id, 'chapters/figures.tex'), source.replaceAll(privatePath, asset));
  assert.ok(!(await store.files(project.id)).some((name: string) => name.includes(reference.token)));
  assert.deepEqual(await fs.readFile(path.join(store.projectDir(project.id), '.refract-attachments', used.token)), bytes);
  await materializeDocumentImages(store, project.id);
  assert.equal((await store.files(project.id)).filter((name: string) => name === asset).length, 1);
  const copy = await store.duplicate(project.id);
  assert.deepEqual(await fs.readFile(await store.existingFile(copy.id, asset)), bytes);
  assert.ok(JSON.stringify(await store.tree(project.id)).includes(asset.split('/')[1]));
});

test('reference-only images and commented or literal examples stay hidden', async t => {
  const {store, project, used} = await fixture(t);
  const command = `\\includegraphics{.refract-attachments/${used.token}}`;
  const source = `% ${command}\n\\begin{verbatim}\n${command}\n\\end{verbatim}\n\\verb|${command}|\n`;
  await store.write(project.id, 'main.tex', source);
  await materializeDocumentImages(store, project.id);
  assert.deepEqual(await store.files(project.id), ['main.tex']);
  assert.equal(await store.read(project.id, 'main.tex'), source);
  await store.write(project.id, 'main.tex', command);
  await store.write(project.id, 'chapters/figures.tex', command);
  await materializeDocumentImages(store, project.id);
  const expected = `\\includegraphics{images/chat-${used.token}}`;
  assert.equal(await store.read(project.id, 'main.tex'), expected);
  assert.equal(await store.read(project.id, 'chapters/figures.tex'), expected);
});

test('a later assistant turn materializes an earlier attachment even after cancellation', async t => {
  const {store, project, used} = await fixture(t);
  const service = new ChatService(store, async () => {
    await store.write(project.id, 'main.tex', `\\includegraphics{.refract-attachments/${used.token}}`);
    throw new Error('Assistant operation cancelled.');
  });
  const chat = await service.send(project.id, 'Use the earlier screenshot');
  assert.equal(chat.messages.at(-1).role, 'error');
  assert.match(await store.read(project.id, 'main.tex'), /images\/chat-/);
  assert.equal((await store.files(project.id)).length, 2);
});

test('promotion refuses conflicting assets and linked private images', async t => {
  const {store, project, used} = await fixture(t);
  const source = `\\includegraphics{.refract-attachments/${used.token}}`;
  await store.write(project.id, 'main.tex', source);
  await store.write(project.id, `images/chat-${used.token}`, 'existing content');
  await assert.rejects(materializeDocumentImages(store, project.id), /already exists/);
  assert.equal(await store.read(project.id, 'main.tex'), source);
  assert.equal(await store.read(project.id, `images/chat-${used.token}`), 'existing content');
  const privateFile = path.join(store.projectDir(project.id), '.refract-attachments', used.token);
  await fs.link(privateFile, path.join(store.projectDir(project.id), 'linked.png'));
  await assert.rejects(materializeDocumentImages(store, project.id), /Invalid document image/);
});
