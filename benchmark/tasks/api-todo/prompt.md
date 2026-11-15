Add a JSON API for todos under `/api/todos` in this Next.js App Router project. Use an in-memory store and do not add dependencies or change `package.json`. Do not add route segment config such as `export const dynamic`; the scaffold enables Next.js cache components, which rejects it. Read the request URL in GET so the handler runs at request time.

Contract:

- `GET /api/todos` returns `200` with `{"todos":[{"id":string,"text":string,"completed":boolean}]}`
- `POST /api/todos` with `{"text":string}` returns `201` with `{"todo":{...}}`; empty or missing text returns `400`
- `PATCH /api/todos` with `{"id":string,"completed":boolean}` returns `200` with the updated `{"todo":{...}}`
- `DELETE /api/todos?id=<id>` returns `200` with `{"deleted":true}`
- Unknown ids return `404`

Use a unique string id for each todo. Make sure `npm run build` succeeds. When you are done, reply with a one-line summary.
