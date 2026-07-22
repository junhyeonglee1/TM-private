import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Pairing {
  id: string;
  deviceLabel: string;
  status: "pending" | "approved";
  requestedAt: string;
  expiresAt: string;
}

interface RegisteredDevice {
  id: string;
  label: string;
  status: "active" | "revoked" | "expired";
  createdAt: string;
  lastSeenAt: string;
  expiresAt: string;
  revokedAt: string | null;
}

interface Collection<T> { items: T[]; }

const message = (error: unknown) =>
  error instanceof Error ? error.message : typeof error === "string" ? error : "기기 관리 요청에 실패했습니다.";

const time = (value: string) => new Intl.DateTimeFormat("ko-KR", {
  dateStyle: "medium",
  timeStyle: "short",
  timeZone: "Asia/Seoul",
}).format(new Date(value));

async function admin<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>("invoke_cloud_device_admin", { command, args });
}

export function DeviceManagementPage() {
  const [pairings, setPairings] = useState<Pairing[]>([]);
  const [devices, setDevices] = useState<RegisteredDevice[]>([]);
  const [codes, setCodes] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [pairingData, deviceData] = await Promise.all([
        admin<Collection<Pairing>>("list_pairings"),
        admin<Collection<RegisteredDevice>>("list_devices"),
      ]);
      setPairings(pairingData.items);
      setDevices(deviceData.items);
      setError(null);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const approve = async (pairing: Pairing) => {
    const code = codes[pairing.id]?.trim() ?? "";
    if (!/^\d{6}$/.test(code)) {
      setError("모바일에 표시된 6자리 코드를 정확히 입력하세요.");
      return;
    }
    setWorking(pairing.id);
    try {
      await admin("approve_pairing", { pairingId: pairing.id, code });
      setCodes((current) => ({ ...current, [pairing.id]: "" }));
      await load();
    } catch (reason) {
      setError(message(reason));
    } finally {
      setWorking(null);
    }
  };

  const revoke = async (device: RegisteredDevice) => {
    if (!window.confirm(`${device.label} 기기의 TM 접근을 즉시 해제할까요?`)) return;
    setWorking(device.id);
    try {
      await admin("revoke_device", { deviceId: device.id });
      await load();
    } catch (reason) {
      setError(message(reason));
    } finally {
      setWorking(null);
    }
  };

  const revokeAll = async () => {
    const active = devices.filter((device) => device.status === "active").length;
    if (!active || !window.confirm(`활성 기기 ${active}대의 접근을 모두 즉시 해제할까요?`)) return;
    setWorking("all");
    try {
      await admin("revoke_all_devices");
      await load();
    } catch (reason) {
      setError(message(reason));
    } finally {
      setWorking(null);
    }
  };

  return (
    <section className="page device-management">
      <header className="page-header">
        <div>
          <span className="eyebrow">Cloud security</span>
          <h1>기기 관리</h1>
          <p>모바일의 이름과 6자리 코드를 직접 확인한 뒤 승인하세요. 토큰·위치·브라우저 지문은 표시하거나 수집하지 않습니다.</p>
        </div>
        <button className="secondary-button" disabled={loading} onClick={() => void load()} type="button">새로고침</button>
      </header>

      {error && <div className="inline-alert" role="alert">{error}</div>}

      <div className="device-section-heading">
        <div><h2>승인 대기</h2><p>코드는 요청 후 10분 동안만 유효합니다.</p></div>
        <span>{pairings.length}</span>
      </div>
      <div className="device-grid">
        {pairings.map((pairing) => (
          <article className="device-card" key={pairing.id}>
            <div className="device-card__title"><strong>{pairing.deviceLabel}</strong><span>{pairing.status}</span></div>
            <dl><dt>요청</dt><dd>{time(pairing.requestedAt)}</dd><dt>만료</dt><dd>{time(pairing.expiresAt)}</dd></dl>
            {pairing.status === "pending" ? (
              <div className="pairing-approval">
                <label htmlFor={`pairing-${pairing.id}`}>모바일 6자리 코드</label>
                <input
                  id={`pairing-${pairing.id}`}
                  inputMode="numeric"
                  maxLength={6}
                  onChange={(event) => setCodes((current) => ({ ...current, [pairing.id]: event.target.value.replace(/\D/g, "") }))}
                  placeholder="000000"
                  value={codes[pairing.id] ?? ""}
                />
                <button className="primary-button" disabled={working === pairing.id} onClick={() => void approve(pairing)} type="button">이 기기 승인</button>
              </div>
            ) : <p className="device-card__notice">승인됨 · 모바일에서 ‘승인했어요’를 눌러 등록을 완료하세요.</p>}
          </article>
        ))}
        {!loading && pairings.length === 0 && <div className="device-empty">대기 중인 기기 등록 요청이 없습니다.</div>}
      </div>

      <div className="device-section-heading device-section-heading--devices">
        <div><h2>등록된 기기</h2><p>폐기하면 해당 기기의 다음 요청부터 401로 차단됩니다.</p></div>
        <button className="danger-button" disabled={!devices.some((device) => device.status === "active") || working === "all"} onClick={() => void revokeAll()} type="button">활성 기기 전체 폐기</button>
      </div>
      <div className="device-grid">
        {devices.map((device) => (
          <article className={`device-card device-card--${device.status}`} key={device.id}>
            <div className="device-card__title"><strong>{device.label}</strong><span>{device.status}</span></div>
            <dl>
              <dt>등록</dt><dd>{time(device.createdAt)}</dd>
              <dt>최근 사용</dt><dd>{time(device.lastSeenAt)}</dd>
              <dt>만료</dt><dd>{time(device.expiresAt)}</dd>
            </dl>
            {device.status === "active" && <button className="danger-button" disabled={working === device.id} onClick={() => void revoke(device)} type="button">이 기기 접근 폐기</button>}
          </article>
        ))}
        {!loading && devices.length === 0 && <div className="device-empty">아직 등록된 모바일 기기가 없습니다.</div>}
      </div>
    </section>
  );
}
