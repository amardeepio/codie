import assert from "node:assert/strict";
import test from "node:test";

import { filterTodos, remaining, toggle } from "./src/lib/todos.ts";

const todos = [
  { id: "1", text: "Buy milk", completed: false },
  { id: "2", text: "Walk dog", completed: true },
];

test("hidden: toggle flips the matching todo and leaves others alone", () => {
  const next = toggle(todos, "2");
  assert.equal(next[0].completed, false);
  assert.equal(next[1].completed, false);
  assert.notEqual(next[1], todos[1], "toggle must return a new object");
});

test("hidden: toggle is a no-op for an unknown id", () => {
  const next = toggle(todos, "missing");
  assert.deepEqual(next, todos);
});

test("hidden: active and completed partition the list", () => {
  assert.deepEqual(
    filterTodos(todos, "active").map((todo) => todo.id),
    ["1"],
  );
  assert.deepEqual(
    filterTodos(todos, "completed").map((todo) => todo.id),
    ["2"],
  );
  assert.equal(filterTodos([], "all").length, 0);
});

test("hidden: remaining counts only incomplete todos", () => {
  assert.equal(remaining(todos), 1);
  assert.equal(remaining([]), 0);
  assert.equal(
    remaining([{ id: "x", text: "done", completed: true }]),
    0,
  );
});
