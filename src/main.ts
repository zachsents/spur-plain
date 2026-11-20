// The plain spur: Tasks', Images' and Recorder's own MCP servers (`<app> mcp`, each relaying to its
// running app and launching it in the background when a tool needs it) as one server on stdio.
// Their tools and event types carry the app's prefix (`tasks_tick`, `plain.tasks.data.changed`);
// Tasks' list widget is `ui://plain/tasks`, its taps calling the prefixed tools.
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import path from "node:path"
import { A2UI_MIME_TYPE, EVENTS, isJson, SUBSCRIPTION, VERSION, WIDGET } from "spur-kit"
import type { Json, Widget } from "spur-kit"

import { lines, parse, spawnChild } from "./child.ts"
import type { Child } from "./child.ts"

const APPS = [
  { prefix: "tasks", name: "Tasks" },
  { prefix: "images", name: "Images" },
  { prefix: "recorder", name: "Recorder" },
] as const

const SPUR = "plain"
const WIDGET_URI = `ui://${SPUR}/tasks`
const LIST_URI = "tasks://widget/list"
const SHOW_LIST = "tasks_show_list"

const tasksWidget = {
  name: "tasks",
  title: "Tasks",
  surfaces: ["space", "chat"],
  resource: WIDGET_URI,
  tool: SHOW_LIST,
  refresh: [`${SPUR}.tasks.data.changed`],
} satisfies Widget

const field = (m: Json, key: string): Json => {
  const v = m[key]
  return isJson(v) ? v : {}
}
const list = (v: unknown): readonly Json[] => (Array.isArray(v) ? v.filter(isJson) : [])
const str = (v: unknown) => (typeof v === "string" ? v : "")

const write = (m: Json) => {
  process.stdout.write(`${JSON.stringify(m)}\n`)
}
const answer = (id: unknown, result: Json) => {
  write({ jsonrpc: "2.0", id, result })
}
const fail = (id: unknown, code: number, message: string) => {
  write({ jsonrpc: "2.0", id, error: { code, message } })
}

/** Client request ids by each child's own, for subscriptions' notifications and cancelling. */
const clientIds = new Map<string, unknown>()
const childIds = new Map<unknown, { child: Child; id: number }>()

const heard = (m: Json, child: Child) => {
  const method = str(m.method)
  if (m.id !== undefined && method !== "") {
    child.send(
      method === "ping"
        ? { jsonrpc: "2.0", id: m.id, result: {} }
        : { jsonrpc: "2.0", id: m.id, error: { code: -32_601, message: `No ${method}` } },
    )
    return
  }
  if (method.startsWith("notifications/events/")) {
    const params = field(m, "params")
    const meta = field(params, "_meta")
    const id = clientIds.get(`${child.prefix}:${String(meta[SUBSCRIPTION])}`)
    if (id === undefined) {
      return
    }
    const named =
      typeof params.name === "string" ? { name: `${SPUR}.${child.prefix}.${params.name}` } : {}
    write({ ...m, params: { ...params, ...named, _meta: { ...meta, [SUBSCRIPTION]: id } } })
    return
  }
  if (method === "notifications/progress" || method === "notifications/message") {
    write(m)
  }
}

const dir = process.env.PLAIN_APPS ?? path.join(homedir(), "Applications")
const children = new Map<string, Child>()
for (const app of APPS) {
  const binary = path.join(dir, `${app.name}.app`, "Contents", "MacOS", app.prefix)
  if (existsSync(binary)) {
    children.set(app.prefix, spawnChild(app, binary, heard))
  } else {
    process.stderr.write(`${app.name} isn't installed in ${dir}: bun run apps\n`)
  }
}

/** A request to one child as the client's own: its answer under the client's id. */
const forward = async (child: Child, id: unknown, method: string, params: Json): Promise<Json> => {
  const key = (n: number) => `${child.prefix}:${n}`
  let sentAs = 0
  const response = await child.request(method, params, (n) => {
    sentAs = n
    clientIds.set(key(n), id)
    childIds.set(id, { child, id: n })
  })
  clientIds.delete(key(sentAs))
  childIds.delete(id)
  return { ...response, id }
}

/** Every child's result for one request: {} from one that failed. */
const everyone = async (method: string, params: Json) =>
  await Promise.all(
    [...children.values()].map(async (child) => ({
      child,
      result: field(await child.request(method, params), "result"),
    })),
  )

/** Points a widget's taps at the prefixed tools: an action's event names the tool it calls. */
const prefixActions = (v: unknown, prefix: string): unknown => {
  if (Array.isArray(v)) {
    return v.map((x) => prefixActions(x, prefix))
  }
  if (!isJson(v)) {
    return v
  }
  const out: Json = {}
  for (const [k, x] of Object.entries(v)) {
    out[k] = prefixActions(x, prefix)
  }
  const event = field(field(out, "action"), "event")
  if (typeof event.name === "string") {
    out.action = { ...field(out, "action"), event: { ...event, name: `${prefix}_${event.name}` } }
  }
  return out
}

const decodeMessages = (text: string): readonly unknown[] => {
  try {
    const v: unknown = JSON.parse(text)
    return Array.isArray(v) ? v : []
  } catch {
    return []
  }
}

/** A widget resource's messages with its taps prefixed. */
const widgetContent = (r: Json) => ({
  ...r,
  text: JSON.stringify(prefixActions(decodeMessages(str(r.text)), "tasks")),
})

/** `show_list`'s result: its taps prefixed, and its data model as the structured result. */
const shownList = (result: Json): Json => {
  let data: unknown = {}
  const content = list(result.content).map((c) => {
    const r = field(c, "resource")
    if (c.type !== "resource" || r.mimeType !== A2UI_MIME_TYPE) {
      return c
    }
    for (const m of decodeMessages(str(r.text))) {
      const update = isJson(m) ? field(m, "updateDataModel") : {}
      if (update.value !== undefined) {
        data = update.value
      }
    }
    return { ...c, resource: widgetContent(r) }
  })
  return { ...result, content, structuredContent: data }
}

/** A prefixed name's child and the name it knows: `tasks_tick` is Tasks' `tick`. */
const route = (name: string, separator: string) => {
  const cut = name.indexOf(separator)
  const child = cut === -1 ? undefined : children.get(name.slice(0, cut))
  return child ? { child, name: name.slice(cut + separator.length) } : null
}

const tools = async () => {
  const all = await everyone("tools/list", {})
  return all.flatMap(({ child, result }) =>
    list(result.tools)
      .filter((t) => {
        const seen: unknown = field(field(t, "_meta"), "ui").visibility
        return !Array.isArray(seen) || seen.includes("model")
      })
      .map((t) => {
        const name = `${child.prefix}_${str(t.name)}`
        const { _meta: _, ...rest } = t
        return {
          ...rest,
          name,
          title: `${child.name}: ${str(t.title)}`,
          ...(name === SHOW_LIST && { _meta: { ui: { resourceUri: WIDGET_URI } } }),
        }
      }),
  )
}

const events = async () => {
  const all = await everyone("events/list", {})
  return all.flatMap(({ child, result }) =>
    list(result.events).map((e) => ({
      ...e,
      name: `${SPUR}.${child.prefix}.${str(e.name)}`,
      _meta: { ...field(e, "_meta"), [VERSION]: 1 },
    })),
  )
}

const INSTRUCTIONS =
  "Tasks, Images and Recorder: plain's apps on this computer, their tools named by app (tasks_, images_, recorder_). Changes show in the app at once; an app that isn't running is launched in the background. Every tool returns the app's state after the change. tasks_show_list is a list as a widget."

/** A response with its result changed, if it has one. */
const mapResult = (r: Json, f: (result: Json) => Json): Json =>
  isJson(r.result) ? { ...r, result: f(r.result) } : r

const empty = (id: unknown) => {
  answer(id, {})
}

const initialize = async (id: unknown, params: Json) => {
  const all = await everyone("initialize", params)
  const version = all.map(({ result }) => str(result.protocolVersion)).find((v) => v !== "")
  answer(id, {
    protocolVersion: version ?? str(params.protocolVersion),
    capabilities: { tools: {}, resources: {}, extensions: { [EVENTS]: { listChanged: false } } },
    serverInfo: { name: SPUR, version: "0.1.0" },
    instructions: INSTRUCTIONS,
  })
}

const callTool = async (id: unknown, params: Json) => {
  const name = str(params.name)
  const to = route(name, "_")
  if (!to) {
    answer(id, { content: [{ type: "text", text: `No tool ${name}` }], isError: true })
    return
  }
  const r = await forward(to.child, id, "tools/call", { ...params, name: to.name })
  write(name === SHOW_LIST ? mapResult(r, shownList) : r)
}

const readResource = async (id: unknown, params: Json) => {
  const uri = str(params.uri)
  const tasks = children.get("tasks")
  if (uri === WIDGET_URI && tasks) {
    const r = await forward(tasks, id, "resources/read", { ...params, uri: LIST_URI })
    write(
      mapResult(r, (result) => ({
        ...result,
        contents: list(result.contents).map((c) => ({ ...widgetContent(c), uri })),
      })),
    )
    return
  }
  const to = route(uri, "://")
  if (!to) {
    fail(id, -32_002, `No resource ${uri}`)
    return
  }
  write(await forward(to.child, id, "resources/read", params))
}

const eventRequest = (method: string) => async (id: unknown, params: Json) => {
  const to = route(str(params.name).replace(`${SPUR}.`, ""), ".")
  if (!to) {
    fail(id, -32_023, `No event type ${str(params.name)}`)
    return
  }
  const r = await forward(to.child, id, method, { ...params, name: to.name })
  write(
    mapResult(r, (result) => ({
      ...result,
      ...(Array.isArray(result.events) && {
        events: list(result.events).map((e) => ({
          ...e,
          name: `${SPUR}.${to.child.prefix}.${str(e.name)}`,
        })),
      }),
    })),
  )
}

const requests: Record<string, (id: unknown, params: Json) => void | Promise<void>> = {
  initialize,
  ping: empty,
  "logging/setLevel": empty,
  "resources/subscribe": empty,
  "resources/unsubscribe": empty,
  "tools/list": async (id) => {
    answer(id, { tools: await tools() })
  },
  "tools/call": callTool,
  "resources/list": (id) => {
    const widget = {
      uri: WIDGET_URI,
      name: tasksWidget.name,
      title: tasksWidget.title,
      mimeType: A2UI_MIME_TYPE,
      _meta: { [WIDGET]: tasksWidget },
    }
    answer(id, { resources: children.has("tasks") ? [widget] : [] })
  },
  "resources/templates/list": (id) => {
    answer(id, { resourceTemplates: [] })
  },
  "resources/read": readResource,
  "events/list": async (id) => {
    answer(id, { events: await events() })
  },
  "events/stream": eventRequest("events/stream"),
  "events/poll": eventRequest("events/poll"),
}

const notifications: Record<string, (m: Json) => void> = {
  "notifications/initialized": (m) => {
    for (const child of children.values()) {
      child.send(m)
    }
  },
  "notifications/cancelled": (m) => {
    const params = field(m, "params")
    const sent = childIds.get(params.requestId)
    if (sent) {
      sent.child.send({ ...m, params: { ...params, requestId: sent.id } })
    }
  },
}

const handle = async (m: Json) => {
  const method = str(m.method)
  if (m.id === undefined) {
    notifications[method]?.(m)
    return
  }
  const run = requests[method]
  if (run) {
    await run(m.id, field(m, "params"))
  } else {
    fail(m.id, -32_601, `No method ${method}`)
  }
}

await lines(Bun.stdin.stream(), (line) => {
  const m = parse(line)
  if (m) {
    void handle(m)
  }
})
for (const child of children.values()) {
  child.kill()
}
