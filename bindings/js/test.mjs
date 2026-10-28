import test from 'node:test';
import assert from 'node:assert/strict';
import init, { listConflicts, resolveConflict, GitHubCms } from './index.js';

await init();

function createMockFetch(responses) {
  let callIndex = 0;
  return async function mockFetch(url, init) {
    if (callIndex >= responses.length) {
      throw new Error(`Unexpected fetch call #${callIndex} to ${url}`);
    }
    const spec = responses[callIndex++];
    if (spec.error) {
      throw spec.error;
    }
    const status = spec.status ?? 200;
    const headersMap = new Map();
    for (const [k, v] of Object.entries(spec.headers ?? { 'content-type': 'application/json' })) {
      headersMap.set(k.toLowerCase(), v);
    }
    const bodyStr = typeof spec.body === 'string' ? spec.body : JSON.stringify(spec.body ?? {});
    const bodyBytes = Buffer.from(bodyStr);

    return {
      status,
      headers: {
        entries() {
          return headersMap.entries();
        },
        get(key) {
          return headersMap.get(key.toLowerCase());
        },
      },
      async arrayBuffer() {
        return bodyBytes.buffer.slice(
          bodyBytes.byteOffset,
          bodyBytes.byteOffset + bodyBytes.byteLength,
        );
      },
      async text() {
        return bodyStr;
      },
    };
  };
}

test('listConflicts and resolveConflict on block, inline, and scalar conflicts', () => {
  const blockSource = `=[ Title ]

@conflict(
  a: | Section A
  b: | Section B
)

Final paragraph.
`;

  const blockConflicts = listConflicts(blockSource);
  assert.equal(blockConflicts.length, 1);
  assert.equal(blockConflicts[0].index, 0);
  assert.equal(blockConflicts[0].kind, 'block');
  assert.equal(blockConflicts[0].a, 'Section A');
  assert.equal(blockConflicts[0].b, 'Section B');

  const resolvedA = resolveConflict(blockSource, 0, 'a');
  assert.equal(
    resolvedA,
    `=[ Title ]

Section A

Final paragraph.
`,
  );

  const resolvedB = resolveConflict(blockSource, 0, 'b');
  assert.equal(
    resolvedB,
    `=[ Title ]

Section B

Final paragraph.
`,
  );

  const inlineSource = `Paragraph with @conflict(a: [left inline], b: [right inline]) text.`;
  const inlineConflicts = listConflicts(inlineSource);
  assert.equal(inlineConflicts.length, 1);
  assert.equal(inlineConflicts[0].kind, 'inline');
  assert.equal(inlineConflicts[0].a, 'left inline');
  assert.equal(inlineConflicts[0].b, 'right inline');

  const resolvedInlineA = resolveConflict(inlineSource, 0, 'a');
  assert.equal(resolvedInlineA, `Paragraph with left inline text.`);

  const scalarSource = `::meta{
  title: @conflict(a: "Alpha", b: "Beta")
}
`;
  const scalarConflicts = listConflicts(scalarSource);
  assert.equal(scalarConflicts.length, 1);
  assert.equal(scalarConflicts[0].kind, 'scalar');
  assert.equal(scalarConflicts[0].a, 'Alpha');
  assert.equal(scalarConflicts[0].b, 'Beta');

  const resolvedScalarB = resolveConflict(scalarSource, 0, 'b');
  assert.equal(
    resolvedScalarB,
    `::meta{
  title: Beta
}
`,
  );
});

test('resolveConflict throws with structured error codes', () => {
  const source = `=[ Title ]\n\nBody text.\n`;

  assert.throws(
    () => resolveConflict(source, 0, 'a'),
    (err) => err.code === 'IndexOutOfBounds',
  );

  const conflictSource = `@conflict(a: [A], b: [B])`;
  assert.throws(
    () => resolveConflict(conflictSource, 0, 'c'),
    (err) => err.code === 'InvalidSide',
  );

  const malformed = `::meta{ unclosed`;
  assert.throws(
    () => listConflicts(malformed),
    (err) => err.code === 'ConflictParseError',
  );
});

test('GitHubCms: getHead, createFile success and AlreadyExists error', async () => {
  const mockFetch = createMockFetch([
    // 1. getHead
    {
      body: {
        data: {
          repository: {
            ref: {
              target: { oid: 'head-001' },
            },
          },
        },
      },
    },
    // 2. createFile (calls get_head)
    {
      body: {
        data: {
          repository: {
            ref: {
              target: { oid: 'head-001' },
            },
          },
        },
      },
    },
    // 3. createFile (reads remote file at head-001: returns null)
    {
      body: {
        data: {
          repository: {
            object: null,
          },
        },
      },
    },
    // 4. createFile (commitFile createCommitOnBranch)
    {
      body: {
        data: {
          createCommitOnBranch: {
            commit: { oid: 'head-002' },
          },
        },
      },
    },
    // 5. createFile that already exists: get_head
    {
      body: {
        data: {
          repository: {
            ref: {
              target: { oid: 'head-002' },
            },
          },
        },
      },
    },
    // 6. createFile that already exists: read_file returns existing blob
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: 'existing content',
              byteSize: 16,
            },
          },
        },
      },
    },
  ]);

  const cms = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'dummy-token',
    fetch: mockFetch,
  });

  const head = await cms.getHead();
  assert.equal(head, 'head-001');

  const created = await cms.createFile({
    path: 'docs/intro.tmt',
    content: '=[ Hello ]\n\nWorld.\n',
    message: 'docs: create intro',
  });
  assert.equal(created.revision, 'head-002');
  assert.equal(created.hasConflicts, false);
  assert.deepEqual(created.warnings, []);

  await assert.rejects(
    async () => {
      await cms.createFile({
        path: 'docs/intro.tmt',
        content: '=[ Hello ]\n\nOther content.\n',
        message: 'docs: duplicate create',
      });
    },
    (err) => err.code === 'AlreadyExists',
  );
});

test('GitHubCms: readFile absent returns null, present returns content', async () => {
  const mockFetch = createMockFetch([
    // read absent
    {
      body: {
        data: {
          repository: {
            object: null,
          },
        },
      },
    },
    // read present
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: '=[ Note ]\n',
              byteSize: 9,
            },
          },
        },
      },
    },
  ]);

  const cms = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'dummy-token',
    fetch: mockFetch,
  });

  const absent = await cms.readFile('missing.tmt', 'rev-1');
  assert.equal(absent, null);

  const present = await cms.readFile('present.tmt', 'rev-2');
  assert.deepEqual(present, {
    content: '=[ Note ]\n',
    revision: 'rev-2',
  });
});

test('GitHubCms: editFile clean 3-way merge and conflicting merge', async () => {
  const baseContent = `=[ Doc ]

==[ Section A ]
Base A

==[ Section B ]
Base B
`;

  const localContent = `=[ Doc ]

==[ Section A ]
Local edit in A

==[ Section B ]
Base B
`;

  const remoteContent = `=[ Doc ]

==[ Section A ]
Base A

==[ Section B ]
Remote edit in B
`;

  const mockFetch = createMockFetch([
    // 1. editFile clean merge: get_head
    {
      body: {
        data: {
          repository: {
            ref: {
              target: { oid: 'head-remote' },
            },
          },
        },
      },
    },
    // 2. read remote at head-remote
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: remoteContent,
              byteSize: remoteContent.length,
            },
          },
        },
      },
    },
    // 3. read base at base-rev
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: baseContent,
              byteSize: baseContent.length,
            },
          },
        },
      },
    },
    // 4. commitFile
    {
      body: {
        data: {
          createCommitOnBranch: {
            commit: { oid: 'head-merged' },
          },
        },
      },
    },
    // --- Conflicting merge test ---
    // 5. get_head
    {
      body: {
        data: {
          repository: {
            ref: {
              target: { oid: 'head-remote-2' },
            },
          },
        },
      },
    },
    // 6. read remote (same section modified)
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: `=[ Doc ]\n\nRemote conflict in section\n`,
              byteSize: 35,
            },
          },
        },
      },
    },
    // 7. read base
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: `=[ Doc ]\n\nBase section\n`,
              byteSize: 22,
            },
          },
        },
      },
    },
    // 8. commitFile with conflict marker
    {
      body: {
        data: {
          createCommitOnBranch: {
            commit: { oid: 'head-conflict-committed' },
          },
        },
      },
    },
  ]);

  const cms = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'dummy-token',
    fetch: mockFetch,
  });

  const cleanResult = await cms.editFile({
    path: 'doc.tmt',
    baseRev: 'base-rev',
    content: localContent,
    message: 'edit: modify A',
  });
  assert.equal(cleanResult.revision, 'head-merged');
  assert.equal(cleanResult.hasConflicts, false);

  const conflictResult = await cms.editFile({
    path: 'doc.tmt',
    baseRev: 'base-rev',
    content: `=[ Doc ]\n\nLocal conflict in section\n`,
    message: 'edit: conflicting change',
  });
  assert.equal(conflictResult.revision, 'head-conflict-committed');
  assert.equal(conflictResult.hasConflicts, true);
});

test('GitHubCms: BranchMoved retry flow', async () => {
  const baseContent = `=[ Doc ]\n\nLine 1\n`;
  const localContent = `=[ Doc ]\n\nLine 1 edited\n`;

  const mockFetch = createMockFetch([
    // Attempt 1:
    // 1. get_head -> head-1
    { body: { data: { repository: { ref: { target: { oid: 'head-1' } } } } } },
    // 2. read remote at head-1 (remote equals base)
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: baseContent,
              byteSize: baseContent.length,
            },
          },
        },
      },
    },
    // 3. read base at baseRev
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: baseContent,
              byteSize: baseContent.length,
            },
          },
        },
      },
    },
    // 4. commitFile fails with GraphQL error
    {
      body: {
        errors: [{ message: 'Head branch was modified' }],
      },
    },
    // 5. GitHubHost checks if branch moved by querying head: returns head-2 (moved!)
    { body: { data: { repository: { ref: { target: { oid: 'head-2' } } } } } },
    // Attempt 2 (retried automatically by save):
    // 6. get_head -> head-2
    { body: { data: { repository: { ref: { target: { oid: 'head-2' } } } } } },
    // 7. read remote at head-2 (still equals base)
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: baseContent,
              byteSize: baseContent.length,
            },
          },
        },
      },
    },
    // 8. read base at baseRev
    {
      body: {
        data: {
          repository: {
            object: {
              __typename: 'Blob',
              isBinary: false,
              isTruncated: false,
              text: baseContent,
              byteSize: baseContent.length,
            },
          },
        },
      },
    },
    // 9. commitFile succeeds on top of head-2!
    {
      body: {
        data: {
          createCommitOnBranch: {
            commit: { oid: 'head-3' },
          },
        },
      },
    },
  ]);

  const cms = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'dummy-token',
    fetch: mockFetch,
  });

  const res = await cms.editFile({
    path: 'doc.tmt',
    baseRev: 'head-1',
    content: localContent,
    message: 'edit: retry test',
  });
  assert.equal(res.revision, 'head-3');
});

test('GitHubCms: HTTP 401 Unauthorized, HTTP 429 RateLimited, and NonTmtPath', async () => {
  const mock401 = createMockFetch([
    {
      status: 401,
      body: { message: 'Bad credentials' },
    },
  ]);

  const cms401 = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'bad-token',
    fetch: mock401,
  });

  await assert.rejects(
    async () => {
      await cms401.getHead();
    },
    (err) => err.code === 'Unauthorized',
  );

  const mock429 = createMockFetch([
    {
      status: 429,
      body: { message: 'Rate limit exceeded' },
    },
  ]);

  const cms429 = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'good-token',
    fetch: mock429,
  });

  await assert.rejects(
    async () => {
      await cms429.getHead();
    },
    (err) => err.code === 'RateLimited',
  );

  // NonTmtPath validation
  const cmsPlain = new GitHubCms({
    owner: 'test-owner',
    repo: 'test-repo',
    token: 'token',
    fetch: async () => assert.fail('should not reach network'),
  });

  await assert.rejects(
    async () => {
      await cmsPlain.createFile({
        path: 'page.md',
        content: 'hello',
        message: 'invalid path',
      });
    },
    (err) => err.code === 'NonTmtPath',
  );
});
