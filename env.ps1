# Windows(PowerShell)용 로컬 개발 환경 변수.
#
# POSIX 셸(bash/zsh, `env` 파일)과 문법이 달라(export/case 없음) 별도 파일로 둔다.
# 이 저장소 루트에서 아래처럼 "닷소싱"해야 현재 세션에 반영된다 (그냥 실행하면
# 하위 프로세스에만 설정되고 끝난다):
#
#   . .\env.ps1
#
# 사전 준비 (README.md 수동 테스트 절 참조):
#   - PDFium: https://github.com/bblanchon/pdfium-binaries 릴리스에서
#     pdfium-win-x64.zip(또는 .tgz)을 받아 .pdfium\ 에 압축 해제
#   - Kiwi:   https://github.com/bab2min/Kiwi 릴리스에서 정확히 v0.22.2의
#     kiwi_win_x64_v0.22.2.zip(라이브러리) + kiwi_model_v0.22.2_base.tgz(모델)를
#     받아 .kiwi\ 에 압축 해제 (⚠️ v0.23.2는 kiwi-rs와 ABI 불일치로 크래시 —
#     `11_Implementation_Plan.md` 참조)

$repoRoot = $PSScriptRoot

# ⚠️ 확인 필요: pdfium-binaries Windows 배포판의 실제 내부 폴더명(bin/ vs lib/)을
# 검증하지 못했다 (macOS/Linux 배포판은 lib/pdfium.{so,dylib} 확인됨 — 문서화 안 됨).
# 압축을 풀어 pdfium.dll이 어느 폴더에 있는지 확인한 뒤 아래 경로를 맞게 고칠 것.
$env:KNOWDESK_PDFIUM_LIB_DIR = Join-Path $repoRoot ".pdfium\bin"

# kiwi-rs 설치 스크립트(scripts/install_kiwi.ps1)가 실제로 "lib\kiwi.dll"에 두는
# 것을 확인했다 — mac/Linux처럼 "lib" 접두사가 붙지 않는다.
$env:KNOWDESK_KIWI_LIB_PATH = Join-Path $repoRoot ".kiwi\lib\kiwi.dll"
$env:KNOWDESK_KIWI_MODEL_DIR = Join-Path $repoRoot ".kiwi\models\cong\base"
