The todo logic in `src/lib/todos.ts` has bugs. The failing tests in `src/lib/todos.test.ts` describe the expected behavior.

- Fix `src/lib/todos.ts` so the tests pass
- Do not modify the tests, the public function signatures, or `package.json`
- Do not add dependencies

You can run the tests with:

```bash
node --experimental-strip-types --test src/lib/todos.test.ts
```

Make sure `npm run build` still succeeds. When you are done, reply with a one-line summary.
