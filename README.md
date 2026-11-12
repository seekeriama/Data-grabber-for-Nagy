# FourPointProbe (Rust)

Viết lại chương trình **FourPointProbe_Handheld.exe** (VB.NET, 2023) bằng Rust, giữ nguyên cách đo của bản gốc và thêm chức năng chọn **định dạng file CSV: Default / ERS / Sequoia**.

Chương trình chạy được với hai loại máy, tự dò trên các cổng COM khi khởi động:

| Máy | Cổng | Ghi chú |
|---|---|---|
| Keithley SourceMeter + đầu đo 4 điểm | 57600 baud, 8N1, XON/XOFF | lệnh SCPI, nhận dạng bằng `*IDN?` có chữ KEITHLEY |
| Nagy | 9600 baud, 8N1 | bắt tay `£` + `l0`, máy tự gửi kết quả `R=...` |

## Build trên Windows

1. Cài Rust: https://rustup.rs (chọn mặc định; nếu được hỏi thì cài thêm *Visual Studio Build Tools* với "Desktop development with C++").
2. Mở Command Prompt trong thư mục này rồi chạy:
   ```
   cargo build --release
   ```
3. File chạy: `target\release\FourPointProbe.exe`. Chỉ cần chép file này sang máy đo (không cần cài gì thêm).

## Build tự động trên GitHub (không cần cài Rust)

File `.github/workflows/build-windows.yml` build exe trên máy Windows của GitHub:

1. Tạo một repository trên GitHub (nên để **Private**), đẩy toàn bộ thư mục này lên (gồm cả `.github`, `.cargo`, `Cargo.lock`).
2. Mỗi lần push lên nhánh `main`, GitHub tự chạy kiểm thử rồi build. Có thể chạy tay: tab **Actions** → **Build Windows** → **Run workflow**.
3. Khi lượt chạy có dấu tích xanh, mở lượt chạy đó, kéo xuống mục **Artifacts**, tải `FourPointProbe-windows-x64` (file zip chứa exe + README).
4. Muốn có bản phát hành cố định: `git tag v1.0.0` rồi `git push --tags`. GitHub tạo mục **Releases** kèm file `FourPointProbe-windows-x64.zip`.

Exe được build kèm thư viện C runtime (`.cargo/config.toml`) nên chạy được trên máy chưa cài Visual C++ Redistributable.

Chạy thử không cần máy đo (để đào tạo hoặc kiểm tra giao diện):
```
FourPointProbe.exe --simulate nagy
FourPointProbe.exe --simulate 4pp
```

## Định dạng file CSV (menu **CSV Format**)

Có 3 ô chọn; bắt buộc chọn ít nhất 1, chọn được cả 3. Mỗi lần **Submit**, kết quả được ghi vào **tất cả** các file đã chọn, nội dung giống hệt nhau, chỉ khác tên file:

| Định dạng | Nagy 5 điểm | Nagy 3 điểm | Nagy Continuous | 4-Point Probe |
|---|---|---|---|---|
| Default | `Five-Points-20261017.csv` | `Three-Point-20261017.csv` | `Continuous-20261017.csv` | `20261017.csv` |
| ERS | `ERS_Five-Points-20261017.csv` | `ERS_Three-Point-…` | `ERS_Continuous-…` | `ERS_20261017.csv` |
| Sequoia | `Sequoia_Five-Points-20261017.csv` | `Sequoia_Three-Point-…` | `Sequoia_Continuous-…` | `Sequoia_20261017.csv` |

Ví dụ một dòng Nagy 5 điểm (đúng cột như file mẫu):
```
ProductLine,Measurement time,RunNumber,RunMinute,Offset,LEMeasurement,LCMeasurement,CCMeasurement,RCMeasurement,REMeasurement
VG1,17/10/2026 19:46:33,1981,29,-0.4,17.02,17.31,17.3,17.25,16.54
```
RunMinute ghi dạng số (không có `="..."`).

**File đang mở trong Excel:** Windows khóa file nên không ghi được. Chương trình báo, giữ dữ liệu lại (lưu có chữ ký trong Registry để không mất khi tắt máy) và tự ghi bù ngay khi file được đóng. Các file khác vẫn được ghi bình thường.

## Cài đặt được lưu ở đâu (không có file cấu hình)

Chương trình **không tạo file nào cạnh exe**. Cài đặt (thư mục lưu, Line, máy dùng lần trước, **Offset Nagy**, hệ số **3 Point Calibration**, định dạng CSV) nằm trong Registry của Windows:

```
HKEY_CURRENT_USER\Software\Pilkington\FourPointProbe   (giá trị Settings, Pending)
```

- Mỗi giá trị có **chữ ký điện tử (HMAC-SHA256)**. Sửa trực tiếp bằng regedit làm chữ ký sai: chương trình phát hiện, bỏ toàn bộ cài đặt đó (Offset về 0) và báo người dùng nhập lại qua giao diện, có mật khẩu.
- Đổi Offset Nagy và 3 Point Calibration đều cần mật khẩu `ZZCARROT` (hằng số `OFFSET_PASSWORD` trong `src/main.rs`).
- `Pending`: dữ liệu đo chưa ghi được vì file CSV đang mở trong Excel. Cũng có chữ ký, nên không ai sửa được số đo đang chờ. Nếu bị sửa, phần dữ liệu đó bị bỏ và chương trình báo.
- Danh mục Line / Product / Test nằm trong exe (`src/catalog.json` nhúng lúc build). Muốn thêm sản phẩm thì sửa file đó rồi build lại.
- Cài đặt gắn với tài khoản Windows đang đăng nhập. Máy dùng chung một tài khoản thì không ảnh hưởng.
- Chế độ `--simulate` dùng cài đặt riêng trong bộ nhớ và ghi CSV vào thư mục tạm (`%TEMP%\FourPointProbe-SIM`), không đụng tới cài đặt và dữ liệu thật.

**Giới hạn:** chữ ký chống được việc sửa cài đặt mà không qua chương trình. Người có quyền quản trị vẫn xóa được khóa Registry (chương trình sẽ về mặc định, Offset = 0). Người có kỹ năng dịch ngược exe về lý thuyết vẫn lấy được khóa ký.

**Lần đầu chạy:** nhập lại Save Folder, Offset (với Nagy) và CSV Format. Các file `fpp_config.json`, `fpp_pending.json` của bản thử trước (nếu có) không còn dùng, xóa được. `calibration.txt` của bản cũ cũng không còn được đọc; với máy 4 điểm, nhập lại hệ số trong 3 Point Calibration.

## Giữ nguyên như bản gốc

- Danh mục 8 line, 30 sản phẩm (4PP) và Nagy có thêm sản phẩm **TZ18** (test 5 Points / 3 Points); còn lại, 14 test, 220 điểm đo (4PP) và 4 line, 10 sản phẩm, 3 test (Nagy), lấy nguyên từ code gốc.
- 4PP: chờ nhấc / đặt đầu đo (2 lần đọc lệch < 1%, test Calibration < 10%), quét 4 mức dòng × 3 lần đọc điện áp, hồi quy, loại khi r < 0.99, hệ số Sample Size (Float Ribbon 1.0 / 4x4 0.9981 / 12x12 0.9997 / 3x3 0.9966), đa thức hiệu chỉnh trong khoảng Low–High. Test "2 Inch / 4 Inch Ohms" dùng giá trị điện trở trung bình của Keithley.
- Kết quả 4PP có điểm "Not Stored" (Continuous, Gage R and R, Calibration) ghi vào `C:\Records\` như cũ.
- Line OT1: **Get Host Data** đọc `http://ot01_h1/reports/samples/sample.flaws.00`.
- Bấm vào một kết quả để đo lại điểm đó.

## Khác với bản gốc

- Giao diện theo bản cũ: Setup Screen nhỏ gọn, MeasureForm với kết quả bên trái, số đo và biểu đồ bên phải.
- Đổi Offset Nagy và 3 Point Calibration phải nhập mật khẩu (`ZZCARROT`, khai báo ở `OFFSET_PASSWORD` trong `src/main.rs`).
- Sample Size chỉ còn một mục **10cm x 10cm** (hệ số 0.9981, bằng mẫu 4x4 inch của bản gốc). Không ảnh hưởng Nagy; với máy 4 điểm, các test trước đây dùng "Float Ribbon" (hệ số 1.0) nay nhân 0.9981.
- Không còn dùng `calibration.txt`: hệ số 3 Point Calibration lưu cùng cài đặt có chữ ký, đổi phải có mật khẩu.

- Thêm chọn định dạng CSV Default / ERS / Sequoia.
- Dòng CSV không còn dấu phẩy thừa ở cuối, RunMinute ghi dạng số, khớp với file mẫu.
- Sửa lỗi: đo lại một điểm **giữa chừng** làm bản gốc treo (4PP) hoặc ghi tên điểm thành "Continue" (Nagy). Bản mới đo lại xong thì tiếp tục đúng điểm đang đo.
- Không có cổng COM hoặc không tìm thấy máy: hiện thông báo và nút **Retry connection** thay vì tự thoát.
- Hỏi xác nhận khi bấm Exit Measurement mà chưa Submit.

## Kiểm thử

```
cargo test
```
26 bài kiểm thử: đọc số kiểu VB, định dạng CSV và tên file 3 định dạng, ghi bù khi file bị khóa, giao thức Keithley / Nagy, và toàn bộ vòng đo với máy giả lập (đo lại, offset, Continuous, lỗi hồi quy).
