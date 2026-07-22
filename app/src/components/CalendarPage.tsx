import { useCallback, useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";

import { Icon } from "./Icon";
import type {
  CalendarEvent,
  CalendarEventKind,
  CalendarMonth,
  CalendarRecurrence,
  CreateCalendarEventInput,
  UpdateCalendarEventInput,
} from "../types";

interface CalendarPageProps {
  today: string;
  onLoad: (month: string) => Promise<CalendarMonth>;
  onCreate: (input: CreateCalendarEventInput) => Promise<CalendarEvent>;
  onUpdate: (eventId: string, input: UpdateCalendarEventInput) => Promise<CalendarEvent>;
  onDelete: (eventId: string, expectedVersion: number) => Promise<void>;
  onNotify: (message: string, type?: "success" | "error") => void;
}

interface FormState {
  title: string;
  description: string;
  kind: CalendarEventKind;
  startDate: string;
  eventTime: string;
  recurrence: CalendarRecurrence;
  dayOfMonth: string;
  endsOn: string;
}

const weekdayLabels = ["월", "화", "수", "목", "금", "토", "일"];

const monthShift = (month: string, amount: number): string => {
  const [year, monthNumber] = month.split("-").map(Number);
  const shifted = new Date(Date.UTC(year, monthNumber - 1 + amount, 1));
  return `${shifted.getUTCFullYear()}-${String(shifted.getUTCMonth() + 1).padStart(2, "0")}`;
};

const daysInMonth = (month: string): number => {
  const [year, monthNumber] = month.split("-").map(Number);
  return new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
};

const monthLabel = (month: string): string => {
  const [year, monthNumber] = month.split("-").map(Number);
  return `${year}년 ${monthNumber}월`;
};

const recurrenceLabel = (event: Pick<CalendarEvent, "recurrence" | "dayOfMonth">): string => {
  switch (event.recurrence) {
    case "monthly_day": return `매월 ${event.dayOfMonth}일`;
    case "monthly_first_day": return "매월 초일";
    case "monthly_last_day": return "매월 말일";
    default: return "한 번";
  }
};

const emptyForm = (date: string): FormState => ({
  title: "",
  description: "",
  kind: "personal",
  startDate: date,
  eventTime: "",
  recurrence: "none",
  dayOfMonth: String(Number(date.slice(8, 10))),
  endsOn: "",
});

const eventForm = (event: CalendarEvent): FormState => ({
  title: event.title,
  description: event.description,
  kind: event.kind,
  startDate: event.startDate,
  eventTime: event.eventTime?.slice(0, 5) ?? "",
  recurrence: event.recurrence,
  dayOfMonth: event.dayOfMonth === null ? String(Number(event.startDate.slice(8, 10))) : String(event.dayOfMonth),
  endsOn: event.endsOn ?? "",
});

export function CalendarPage({
  today,
  onLoad,
  onCreate,
  onUpdate,
  onDelete,
  onNotify,
}: CalendarPageProps) {
  const [month, setMonth] = useState(today.slice(0, 7));
  const [selectedDate, setSelectedDate] = useState(today);
  const [data, setData] = useState<CalendarMonth | null>(null);
  const [loading, setLoading] = useState(true);
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<CalendarEvent | null>(null);
  const [form, setForm] = useState<FormState>(() => emptyForm(today));
  const [saving, setSaving] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setData(await onLoad(month));
    } catch (error) {
      onNotify(error instanceof Error ? error.message : "캘린더를 불러오지 못했습니다.", "error");
    } finally {
      setLoading(false);
    }
  }, [month, onLoad, onNotify]);

  useEffect(() => { void load(); }, [load]);

  useEffect(() => {
    if (!selectedDate.startsWith(`${month}-`)) setSelectedDate(`${month}-01`);
  }, [month, selectedDate]);

  const occurrencesByDate = useMemo(() => {
    const result = new Map<string, CalendarMonth["occurrences"]>();
    for (const occurrence of data?.occurrences ?? []) {
      const entries = result.get(occurrence.date) ?? [];
      entries.push(occurrence);
      result.set(occurrence.date, entries);
    }
    return result;
  }, [data]);

  const calendarCells = useMemo(() => {
    const [year, monthNumber] = month.split("-").map(Number);
    const leading = (new Date(Date.UTC(year, monthNumber - 1, 1)).getUTCDay() + 6) % 7;
    const count = daysInMonth(month);
    return Array.from({ length: leading + count }, (_, index) => {
      if (index < leading) return null;
      const day = index - leading + 1;
      return `${month}-${String(day).padStart(2, "0")}`;
    });
  }, [month]);

  const selectedOccurrences = occurrencesByDate.get(selectedDate) ?? [];

  const openCreate = (date = selectedDate) => {
    setEditing(null);
    setForm(emptyForm(date));
    setFormOpen(true);
    setConfirmDeleteId(null);
  };

  const openEdit = (event: CalendarEvent) => {
    setEditing(event);
    setForm(eventForm(event));
    setFormOpen(true);
    setConfirmDeleteId(null);
  };

  const closeForm = () => {
    setFormOpen(false);
    setEditing(null);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const input: CreateCalendarEventInput = {
      title: form.title.trim(),
      description: form.description.trim(),
      kind: form.kind,
      startDate: form.startDate,
      eventTime: form.eventTime ? `${form.eventTime}:00` : null,
      recurrence: form.recurrence,
      dayOfMonth: form.recurrence === "monthly_day" ? Number(form.dayOfMonth) : null,
      endsOn: form.recurrence === "none" || !form.endsOn ? null : form.endsOn,
    };
    setSaving(true);
    try {
      if (editing) {
        await onUpdate(editing.id, { ...input, expectedVersion: editing.version });
        onNotify("일정을 변경했습니다.");
      } else {
        await onCreate(input);
        onNotify("일정을 추가했습니다.");
      }
      const targetMonth = input.startDate.slice(0, 7);
      closeForm();
      setMonth(targetMonth);
      setSelectedDate(input.startDate);
      setData(await onLoad(targetMonth));
    } catch (error) {
      onNotify(error instanceof Error ? error.message : "일정을 저장하지 못했습니다.", "error");
    } finally {
      setSaving(false);
    }
  };

  const remove = async (event: CalendarEvent) => {
    if (confirmDeleteId !== event.id) {
      setConfirmDeleteId(event.id);
      return;
    }
    try {
      await onDelete(event.id, event.version);
      setConfirmDeleteId(null);
      onNotify("일정을 삭제했습니다.");
      await load();
    } catch (error) {
      onNotify(error instanceof Error ? error.message : "일정을 삭제하지 못했습니다.", "error");
    }
  };

  const eventById = (eventId: string) => data?.events.find((event) => event.id === eventId);

  return (
    <div className="page-stack calendar-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">개인 일정 · 서울 시간</span>
          <h1>캘린더</h1>
          <p>일회성 일정과 매월 특정일·초일·말일 반복 일정을 관리합니다.</p>
        </div>
        <button className="primary-button" onClick={() => openCreate()} type="button"><Icon name="plus" size={15} /> 일정 추가</button>
      </header>

      <section className="calendar-toolbar" aria-label="월 이동">
        <button aria-label="이전 달" className="icon-button" onClick={() => setMonth(monthShift(month, -1))} type="button"><Icon name="arrow" /></button>
        <h2>{monthLabel(month)}</h2>
        <button aria-label="다음 달" className="icon-button calendar-toolbar__next" onClick={() => setMonth(monthShift(month, 1))} type="button"><Icon name="arrow" /></button>
        <button className="secondary-button secondary-button--small" onClick={() => { setMonth(today.slice(0, 7)); setSelectedDate(today); }} type="button">오늘</button>
      </section>

      <div className="calendar-layout">
        <section className="panel calendar-board" aria-label={`${monthLabel(month)} 달력`}>
          <div className="calendar-weekdays">{weekdayLabels.map((label) => <span key={label}>{label}</span>)}</div>
          <div className="calendar-grid">
            {calendarCells.map((date, index) => date === null
              ? <span className="calendar-day calendar-day--empty" key={`empty-${index}`} />
              : (
                <button
                  aria-label={`${Number(date.slice(8, 10))}일${(occurrencesByDate.get(date)?.length ?? 0) ? ` 일정 ${occurrencesByDate.get(date)?.length}개` : ""}`}
                  aria-pressed={selectedDate === date}
                  className={`calendar-day${date === today ? " calendar-day--today" : ""}`}
                  key={date}
                  onClick={() => setSelectedDate(date)}
                  type="button"
                >
                  <strong>{Number(date.slice(8, 10))}</strong>
                  <span className="calendar-day__events">
                    {(occurrencesByDate.get(date) ?? []).slice(0, 3).map((occurrence) => (
                      <i className={`calendar-event-dot calendar-event-dot--${occurrence.kind}`} key={occurrence.occurrenceKey}>{occurrence.title}</i>
                    ))}
                    {(occurrencesByDate.get(date)?.length ?? 0) > 3 && <small>+{(occurrencesByDate.get(date)?.length ?? 0) - 3}</small>}
                  </span>
                </button>
              ))}
          </div>
          {loading && <div className="calendar-loading" aria-live="polite">일정을 불러오는 중…</div>}
        </section>

        <aside className="panel calendar-agenda">
          <div className="panel__header panel__header--compact">
            <div><span className="eyebrow">선택한 날짜</span><h2>{Number(selectedDate.slice(5, 7))}월 {Number(selectedDate.slice(8, 10))}일</h2></div>
            <button className="secondary-button secondary-button--small" onClick={() => openCreate(selectedDate)} type="button">추가</button>
          </div>
          <div className="calendar-agenda__list">
            {selectedOccurrences.length === 0 && <p className="calendar-empty">등록된 일정이 없습니다.</p>}
            {selectedOccurrences.map((occurrence) => {
              const source = eventById(occurrence.eventId);
              return (
                <article className={`calendar-agenda-item calendar-agenda-item--${occurrence.kind}`} key={occurrence.occurrenceKey}>
                  <div><span>{occurrence.eventTime ? occurrence.eventTime.slice(0, 5) : "하루 종일"}</span><strong>{occurrence.title}</strong><small>{recurrenceLabel({ recurrence: occurrence.recurrence, dayOfMonth: source?.dayOfMonth ?? null })}</small></div>
                  {source && <button aria-label={`${occurrence.title} 편집`} className="icon-button icon-button--small" onClick={() => openEdit(source)} type="button"><Icon name="more" size={15} /></button>}
                </article>
              );
            })}
          </div>
        </aside>
      </div>

      {formOpen && (
        <section className="panel calendar-form-panel" aria-labelledby="calendar-form-title">
          <div className="panel__header">
            <div><span className="eyebrow">{editing ? "일정 편집" : "새 일정"}</span><h2 id="calendar-form-title">{editing ? editing.title : "일정 추가"}</h2></div>
            <button aria-label="일정 입력 닫기" className="icon-button" onClick={closeForm} type="button"><Icon name="close" /></button>
          </div>
          <form className="calendar-form" onSubmit={submit}>
            <label className="field"><span>제목</span><input autoFocus maxLength={200} onChange={(event) => setForm({ ...form, title: event.target.value })} required value={form.title} /></label>
            <label className="field"><span>종류</span><select onChange={(event) => setForm({ ...form, kind: event.target.value as CalendarEventKind })} value={form.kind}><option value="personal">개인 일정</option><option value="payment">납부일</option></select></label>
            <label className="field"><span>날짜·시작일</span><input onChange={(event) => setForm({ ...form, startDate: event.target.value, dayOfMonth: String(Number(event.target.value.slice(8, 10))) })} required type="date" value={form.startDate} /></label>
            <label className="field"><span>시간 (선택)</span><input onChange={(event) => setForm({ ...form, eventTime: event.target.value })} type="time" value={form.eventTime} /></label>
            <label className="field"><span>반복</span><select onChange={(event) => setForm({ ...form, recurrence: event.target.value as CalendarRecurrence })} value={form.recurrence}><option value="none">반복 없음</option><option value="monthly_day">매월 특정일</option><option value="monthly_first_day">매월 초일</option><option value="monthly_last_day">매월 말일</option></select></label>
            {form.recurrence === "monthly_day" && <label className="field"><span>매월 날짜</span><input max={31} min={1} onChange={(event) => setForm({ ...form, dayOfMonth: event.target.value })} required type="number" value={form.dayOfMonth} /><small>31일이 없는 달에는 해당 일정을 건너뜁니다.</small></label>}
            {form.recurrence !== "none" && <label className="field"><span>반복 종료일 (선택)</span><input min={form.startDate} onChange={(event) => setForm({ ...form, endsOn: event.target.value })} type="date" value={form.endsOn} /></label>}
            <label className="field calendar-form__description"><span>메모 (선택)</span><textarea maxLength={4000} onChange={(event) => setForm({ ...form, description: event.target.value })} rows={3} value={form.description} /></label>
            <div className="calendar-form__actions">
              {editing && <button className="danger-button" onClick={() => void remove(editing)} type="button">{confirmDeleteId === editing.id ? "삭제 확인" : "삭제"}</button>}
              <button className="secondary-button" onClick={closeForm} type="button">취소</button>
              <button className="primary-button" disabled={saving} type="submit">{saving ? "저장 중…" : editing ? "변경 저장" : "일정 추가"}</button>
            </div>
          </form>
        </section>
      )}

      <section className="panel calendar-rules">
        <div className="panel__header"><div><span className="eyebrow">반복 관리</span><h2>등록된 반복 일정</h2></div><span className="count-pill">{data?.events.filter((event) => event.recurrence !== "none").length ?? 0}</span></div>
        <div className="calendar-rules__list">
          {(data?.events.filter((event) => event.recurrence !== "none") ?? []).map((event) => (
            <button key={event.id} onClick={() => openEdit(event)} type="button"><i className={`calendar-event-dot calendar-event-dot--${event.kind}`} /><span><strong>{event.title}</strong><small>{recurrenceLabel(event)} · {event.startDate}부터{event.endsOn ? ` ${event.endsOn}까지` : " 계속"}</small></span><Icon name="chevron" size={14} /></button>
          ))}
          {!data?.events.some((event) => event.recurrence !== "none") && <p className="calendar-empty">등록된 반복 일정이 없습니다.</p>}
        </div>
      </section>
    </div>
  );
}
