"use client";

import { useState } from "react";
import styles from "./basic-todo.module.css";

type Todo = {
  id: string;
  text: string;
  completed: boolean;
};

export default function BasicTodo() {
  const [todos, setTodos] = useState<Todo[]>([]);
  const [text, setText] = useState("");

  function addTodo(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = text.trim();
    if (!value) return;
    setTodos((current) => [
      ...current,
      { id: `${Date.now()}-${Math.random().toString(36).slice(2)}`, text: value, completed: false },
    ]);
    setText("");
  }

  function toggleTodo(id: string) {
    setTodos((current) =>
      current.map((todo) =>
        todo.id === id ? { ...todo, completed: !todo.completed } : todo,
      ),
    );
  }

  function deleteTodo(id: string) {
    setTodos((current) => current.filter((todo) => todo.id !== id));
  }

  return (
    <main className={styles.app}>
      <h1 className={styles.title}>Todos</h1>

      <form className={styles.form} onSubmit={addTodo}>
        <input
          className={styles.input}
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder="Add a todo"
          aria-label="New todo"
        />
        <button className={styles.add} type="submit">
          Add
        </button>
      </form>

      <ul className={styles.list}>
        {todos.map((todo) => (
          <li className={styles.item} key={todo.id}>
            <label className={styles.label}>
              <input
                type="checkbox"
                checked={todo.completed}
                onChange={() => toggleTodo(todo.id)}
              />
              <span className={todo.completed ? styles.done : undefined}>{todo.text}</span>
            </label>
            <button
              className={styles.delete}
              type="button"
              aria-label={`Delete ${todo.text}`}
              onClick={() => deleteTodo(todo.id)}
            >
              Delete
            </button>
          </li>
        ))}
      </ul>
    </main>
  );
}
