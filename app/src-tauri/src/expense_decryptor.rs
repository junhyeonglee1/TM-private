use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const MAX_OUTPUT_BYTES: usize = 40 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 4_096;
const DECRYPTOR_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const TIMEOUT_ERROR: &str = "카카오페이 보안 해제 시간이 30초를 초과해 안전하게 중단했습니다.";
const PASSWORD_RETRY_ERROR: &str =
    "TM_EXPENSE_PASSWORD_RETRY:비밀번호가 맞지 않거나 파일이 손상되었습니다.";
const DECRYPTOR_REJECTED_ERROR: &str = "카카오페이 파일을 안전하게 열 수 없습니다.";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Deserialize)]
struct DecryptorHeader {
    ok: bool,
    code: Option<String>,
    length: Option<usize>,
    sha256: Option<String>,
}

pub(crate) fn decrypt_ooxml(encrypted: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let decryptor = locate_decryptor()?;
    verify_decryptor(&decryptor)?;
    let password_bytes = Zeroizing::new(password.as_bytes().to_vec());
    let password_length = u32::try_from(password_bytes.len())
        .map_err(|_| "통합문서 비밀번호가 너무 깁니다.".to_owned())?;
    let mut request = Zeroizing::new(Vec::with_capacity(
        4 + password_bytes.len() + encrypted.len(),
    ));
    request.extend_from_slice(&password_length.to_be_bytes());
    request.extend_from_slice(&password_bytes);
    request.extend_from_slice(encrypted);
    drop(password_bytes);

    let mut command = if decryptor.extension().and_then(|value| value.to_str()) == Some("py") {
        let mut command = Command::new("python.exe");
        command.arg(&decryptor);
        command
    } else {
        Command::new(&decryptor)
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|_| "카카오페이 보안 해제 모듈을 실행할 수 없습니다.".to_owned())?;
    let started = Instant::now();
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = kill_and_reap(&mut child);
            return Err("카카오페이 보안 해제 입력을 열 수 없습니다.".to_owned());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            drop(stdin);
            let _ = kill_and_reap(&mut child);
            return Err("카카오페이 보안 해제 출력을 열 수 없습니다.".to_owned());
        }
    };
    let reader = match spawn_stdout_reader(stdout) {
        Ok(reader) => reader,
        Err(_) => {
            drop(stdin);
            let _ = kill_and_reap(&mut child);
            return Err("카카오페이 보안 해제 출력을 준비할 수 없습니다.".to_owned());
        }
    };
    let writer = match spawn_stdin_writer(stdin, request) {
        Ok(writer) => writer,
        Err(_) => {
            let _ = kill_and_reap(&mut child);
            discard_reader(reader);
            return Err("카카오페이 보안 해제 입력을 준비할 수 없습니다.".to_owned());
        }
    };

    let remaining = DECRYPTOR_TIMEOUT.saturating_sub(started.elapsed());
    let status = match wait_for_exit(&mut child, remaining) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = kill_and_reap(&mut child);
            discard_writer(writer);
            discard_reader(reader);
            return Err(TIMEOUT_ERROR.to_owned());
        }
        Err(_) => {
            let _ = kill_and_reap(&mut child);
            discard_writer(writer);
            discard_reader(reader);
            return Err("카카오페이 보안 해제 결과를 확인할 수 없습니다.".to_owned());
        }
    };
    let write_result = writer.join();
    let read_result = reader.join();
    if !matches!(write_result, Ok(Ok(()))) {
        drop(read_result);
        return Err("카카오페이 보안 해제 입력에 실패했습니다.".to_owned());
    }
    let output = match read_result {
        Ok(Ok(output)) => output,
        Ok(Err(_)) | Err(_) => {
            return Err("카카오페이 보안 해제 출력을 읽을 수 없습니다.".to_owned());
        }
    };
    if output.len() > MAX_OUTPUT_BYTES + MAX_HEADER_BYTES {
        return Err("복호화 결과가 안전 제한을 초과했습니다.".to_owned());
    }
    let newline = output
        .iter()
        .take(MAX_HEADER_BYTES)
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| "카카오페이 보안 해제 응답이 올바르지 않습니다.".to_owned())?;
    let header: DecryptorHeader = serde_json::from_slice(&output[..newline])
        .map_err(|_| "카카오페이 보안 해제 응답이 올바르지 않습니다.".to_owned())?;
    if !header.ok {
        let retryable = matches!(header.code.as_deref(), Some("wrong_password_or_corrupt"));
        return Err(if retryable {
            PASSWORD_RETRY_ERROR.to_owned()
        } else {
            DECRYPTOR_REJECTED_ERROR.to_owned()
        });
    }
    if !status.success() {
        return Err("카카오페이 보안 해제 모듈이 실패했습니다.".to_owned());
    }
    let decrypted = &output[(newline + 1)..];
    if header.length != Some(decrypted.len()) || decrypted.len() > MAX_OUTPUT_BYTES {
        return Err("카카오페이 보안 해제 결과 길이가 올바르지 않습니다.".to_owned());
    }
    let digest = format!("{:x}", Sha256::digest(decrypted));
    if header.sha256.as_deref() != Some(digest.as_str()) {
        return Err("카카오페이 보안 해제 결과 무결성 검증에 실패했습니다.".to_owned());
    }
    Ok(decrypted.to_vec())
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
) -> io::Result<JoinHandle<io::Result<Zeroizing<Vec<u8>>>>> {
    thread::Builder::new()
        .name("tm-expense-decryptor-stdout".to_owned())
        .spawn(move || read_bounded(stdout, MAX_OUTPUT_BYTES + MAX_HEADER_BYTES))
}

fn spawn_stdin_writer(
    mut stdin: ChildStdin,
    request: Zeroizing<Vec<u8>>,
) -> io::Result<JoinHandle<io::Result<()>>> {
    thread::Builder::new()
        .name("tm-expense-decryptor-stdin".to_owned())
        .spawn(move || {
            stdin.write_all(&request)?;
            stdin.flush()?;
            drop(stdin);
            drop(request);
            Ok(())
        })
}

fn read_bounded<R: Read>(reader: R, maximum: usize) -> io::Result<Zeroizing<Vec<u8>>> {
    let read_limit = maximum.saturating_add(1);
    let mut reader = reader.take(u64::try_from(read_limit).unwrap_or(u64::MAX));
    let mut output = Zeroizing::new(Vec::with_capacity(read_limit.min(64 * 1024)));
    reader.read_to_end(&mut output)?;
    Ok(output)
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Ok(None);
        }
        thread::sleep(WAIT_POLL_INTERVAL.min(timeout.saturating_sub(elapsed)));
    }
}

fn kill_and_reap(child: &mut Child) -> io::Result<()> {
    let kill_error = child.kill().err();
    match child.wait() {
        Ok(_) => Ok(()),
        Err(wait_error) => Err(kill_error.unwrap_or(wait_error)),
    }
}

fn discard_writer(writer: JoinHandle<io::Result<()>>) {
    drop(writer.join());
}

fn discard_reader(reader: JoinHandle<io::Result<Zeroizing<Vec<u8>>>>) {
    drop(reader.join());
}

fn locate_decryptor() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("TM_EXPENSE_DECRYPTOR_PATH") {
        return canonical_file(Path::new(&path));
    }
    let executable =
        env::current_exe().map_err(|_| "TM 실행 파일 위치를 확인할 수 없습니다.".to_owned())?;
    if let Some(parent) = executable.parent() {
        let adjacent = parent.join("tm-office-decryptor.exe");
        if adjacent.is_file() {
            return canonical_file(&adjacent);
        }
    }
    #[cfg(debug_assertions)]
    {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../sidecars/tm_office_decryptor.py");
        if script.is_file() {
            return canonical_file(&script);
        }
    }
    Err("카카오페이 보안 해제 모듈이 설치되지 않았습니다.".to_owned())
}

fn canonical_file(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| "카카오페이 보안 해제 모듈을 찾을 수 없습니다.".to_owned())?;
    if !canonical.is_file() {
        return Err("카카오페이 보안 해제 모듈이 파일이 아닙니다.".to_owned());
    }
    Ok(canonical)
}

fn verify_decryptor(path: &Path) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) == Some("py") {
        #[cfg(debug_assertions)]
        return Ok(());
        #[cfg(not(debug_assertions))]
        return Err("운영 환경에서는 스크립트형 보안 해제 모듈을 사용할 수 없습니다.".to_owned());
    }
    let expected = option_env!("TM_EXPENSE_DECRYPTOR_SHA256")
        .filter(|value| value.len() == 64)
        .ok_or_else(|| "보안 해제 모듈의 빌드 해시가 없습니다.".to_owned())?;
    let bytes =
        fs::read(path).map_err(|_| "보안 해제 모듈의 무결성을 확인할 수 없습니다.".to_owned())?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err("보안 해제 모듈의 SHA-256이 빌드 기록과 다릅니다.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, time::Duration};

    #[cfg(windows)]
    use std::process::{Command, Stdio};

    use super::{
        DECRYPTOR_REJECTED_ERROR, DECRYPTOR_TIMEOUT, MAX_HEADER_BYTES, MAX_OUTPUT_BYTES,
        PASSWORD_RETRY_ERROR, TIMEOUT_ERROR, read_bounded,
    };

    #[cfg(windows)]
    use super::{kill_and_reap, wait_for_exit};

    #[test]
    fn decryptor_protocol_limits_are_bounded() {
        assert_eq!(MAX_HEADER_BYTES, 4_096);
        assert_eq!(MAX_OUTPUT_BYTES, 40 * 1024 * 1024);
        assert_eq!(DECRYPTOR_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn stdout_reader_never_buffers_more_than_one_byte_over_limit() -> Result<(), std::io::Error> {
        let output = read_bounded(Cursor::new(vec![7_u8; 64]), 8)?;
        assert_eq!(output.len(), 9);
        assert!(output.iter().all(|byte| *byte == 7));
        Ok(())
    }

    #[test]
    fn security_errors_do_not_echo_sidecar_content() {
        assert_eq!(
            PASSWORD_RETRY_ERROR,
            "TM_EXPENSE_PASSWORD_RETRY:비밀번호가 맞지 않거나 파일이 손상되었습니다."
        );
        assert_eq!(
            DECRYPTOR_REJECTED_ERROR,
            "카카오페이 파일을 안전하게 열 수 없습니다."
        );
        assert!(!TIMEOUT_ERROR.contains("경로"));
        assert!(!TIMEOUT_ERROR.contains("비밀번호"));
    }

    #[cfg(windows)]
    #[test]
    fn timed_out_child_is_killed_and_reaped() -> Result<(), std::io::Error> {
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/C", "ping -n 6 127.0.0.1 >NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        assert!(wait_for_exit(&mut child, Duration::from_millis(20))?.is_none());
        kill_and_reap(&mut child)?;
        assert!(child.try_wait()?.is_some());
        Ok(())
    }
}
