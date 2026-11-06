import { escapeHtml as h, type Copy } from "./contracts";
import { option } from "./views-common";

export type Priority = "high" | "normal" | "low";
export type Schedule =
  | { kind: "immediate" }
  | { kind: "once"; at: number; timezone: string; missed: "runWhenPossible" | "ask"; graceMinutes: number }
  | { kind: "window"; startMinute: number; endMinute: number; weekdays: number[]; timezone: string };

export function scheduleLabel(schedule: Schedule | undefined, t: Copy): string {
  if (!schedule || schedule.kind === "immediate") return t("Cuando sea posible", "When possible");
  if (schedule.kind === "once") return `${t("Programado para", "Scheduled for")} ${new Date(schedule.at).toLocaleString(undefined, { timeZone: "UTC" })} UTC · ${schedule.timezone}${schedule.missed === "ask" ? ` · ${t("preguntar si se retrasa", "ask if missed")}` : ""}`;
  const time = (minute: number) => `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
  const days = [t("Lu", "Mo"), t("Ma", "Tu"), t("Mi", "We"), t("Ju", "Th"), t("Vi", "Fr"), t("Sa", "Sa"), t("Do", "Su")];
  return `${time(schedule.startMinute)} – ${time(schedule.endMinute)} · ${schedule.weekdays.map(day => days[day]).join(", ")} · ${schedule.timezone}`;
}

export function priorityLabel(priority: Priority | undefined, t: Copy): string {
  return priority === "high" ? t("Alta", "High") : priority === "low" ? t("Baja", "Low") : t("Normal", "Normal");
}

export function schedulingFields(prefix: string, priority: Priority, schedule: Schedule, timezone: string, t: Copy): string {
  const localInput = schedule.kind === "once" ? new Date(schedule.at - new Date(schedule.at).getTimezoneOffset() * 60_000).toISOString().slice(0, 16) : "";
  const time = (minute: number) => `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
  return `<label>${t("Prioridad", "Priority")}<select id="${h(prefix)}-priority">${option("high", t("Alta", "High"), priority)}${option("normal", t("Normal", "Normal"), priority)}${option("low", t("Baja", "Low"), priority)}</select></label>
    <label>${t("Inicio", "Start")}<select id="${h(prefix)}-schedule-kind">${option("immediate", t("Cuando sea posible", "When possible"), schedule.kind)}${option("once", t("Fecha y hora", "Date and time"), schedule.kind)}${option("window", t("Franja habitual", "Recurring window"), schedule.kind)}</select></label>
    <div data-schedule-fields="once" ${schedule.kind !== "once" ? "hidden" : ""}><label>${t("Hora local de este equipo", "This computer's local time")}<input id="${h(prefix)}-schedule-at" type="datetime-local" value="${h(localInput)}"></label><label>${t("Si pierde el horario", "If the schedule is missed")}<select id="${h(prefix)}-schedule-missed">${option("runWhenPossible", t("Ejecutar cuando sea posible", "Run when possible"), schedule.kind === "once" ? schedule.missed : "runWhenPossible")}${option("ask", t("Pausar para revisar tras 15 min", "Pause for review after 15 min"), schedule.kind === "once" ? schedule.missed : "runWhenPossible")}</select></label></div>
    <div data-schedule-fields="window" ${schedule.kind !== "window" ? "hidden" : ""}><input id="${h(prefix)}-schedule-timezone" type="hidden" value="${h(schedule.kind === "window" ? schedule.timezone : timezone)}"><div class="form-row"><label>${t("Desde", "From")}<input id="${h(prefix)}-schedule-from" type="time" value="${schedule.kind === "window" ? time(schedule.startMinute) : "22:00"}"></label><label>${t("Hasta", "Until")}<input id="${h(prefix)}-schedule-until" type="time" value="${schedule.kind === "window" ? time(schedule.endMinute) : "06:00"}"></label></div><fieldset><legend>${t("Días de inicio", "Starting days")}</legend>${[t("L", "M"), t("M", "T"), t("X", "W"), t("J", "T"), t("V", "F"), t("S", "S"), t("D", "S")].map((day, i) => `<label><input type="checkbox" name="${h(prefix)}-schedule-day" value="${i}" ${schedule.kind !== "window" || schedule.weekdays.includes(i) ? "checked" : ""}>${day}</label>`).join("")}</fieldset></div>
    <p class="muted">${h(schedule.kind === "immediate" ? timezone : schedule.timezone)} · ${t("Requiere nabnab abierto. Las tareas activas pueden terminar al cerrar la franja.", "Requires nabnab open. Active jobs can finish after the window closes.")}</p>`;
}

export function readScheduling(prefix: string, timezone: string, root: ParentNode = document): { priority: Priority; schedule: Schedule } {
  const value = (suffix: string) => root.querySelector<HTMLInputElement | HTMLSelectElement>(`#${CSS.escape(prefix)}-${suffix}`)?.value ?? "";
  const selectedPriority = value("priority");
  const priority: Priority = selectedPriority === "high" || selectedPriority === "low" ? selectedPriority : "normal";
  switch (value("schedule-kind")) {
    case "once": {
      const at = new Date(value("schedule-at")).getTime();
      if (!Number.isFinite(at)) throw new Error("Choose a valid local date and time.");
      return { priority, schedule: { kind: "once", at, timezone, missed: value("schedule-missed") === "ask" ? "ask" : "runWhenPossible", graceMinutes: 15 } };
    }
    case "window": {
      const minutes = (text: string) => { const [hours, minute] = text.split(":").map(Number); return hours * 60 + minute; };
      const weekdays = [...root.querySelectorAll<HTMLInputElement>(`input[name="${CSS.escape(prefix)}-schedule-day"]:checked`)].map(input => Number(input.value));
      const startMinute = minutes(value("schedule-from")), endMinute = minutes(value("schedule-until"));
      if (!Number.isFinite(startMinute) || !Number.isFinite(endMinute) || startMinute === endMinute || !weekdays.length) throw new Error("Choose different start and end times and at least one day.");
      return { priority, schedule: { kind: "window", startMinute, endMinute, weekdays, timezone: value("schedule-timezone") || timezone } };
    }
    default: return { priority, schedule: { kind: "immediate" } };
  }
}
