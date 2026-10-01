이 프로젝트는 포토샵, 확장자 변환 응용 프로그램 프로젝트입니다.

## 지원하는 파일 변환

| 분야 | 입력 파일 | 변환할 수 있는 형식 |
| --- | --- | --- |
| 이미지 | PNG, JPG/JPEG, GIF, BMP, TIFF/TIF, WEBP, ICO, TGA, PNM, QOI | PNG, JPG/JPEG, GIF, BMP, TIFF/TIF, WEBP, ICO, TGA, PNM, QOI |
| 소리 | WAV, MP3, FLAC, OGG, AAC, M4A, AIFF/AIF, CAF | WAV, FLAC, MP3 |
| 동영상 | MP4, M4V, MOV | MP4, M4V, MOV |

이미지는 정지 이미지만 변환합니다. 움직이는 GIF와 WEBP는 지원하지 않습니다. 소리 파일은 내부 코덱이 지원되는 경우 변환할 수 있으며, 입력 파일을 이용해 확인한 형식은 WAV, MP3, FLAC입니다.

### 문서

| 입력 파일 | 변환할 수 있는 형식 |
| --- | --- |
| TXT | MD, HTML |
| MD | TXT, HTML |
| HTML, HTM | TXT |
| CSV | TSV |
| TSV | CSV |
| PDF | TXT |
| DOCX | TXT, PDF |
| ODT | TXT |

PDF에서 TXT로 변환할 때는 추출 가능한 텍스트만 처리합니다. DOCX와 ODT에서 TXT로 변환할 때도 문서의 텍스트를 추출합니다.

동영상은 H.264 영상과 AAC 소리가 들어 있는 MP4 계열 파일의 형식 식별자를 바꾸며, 영상·소리·재생 시간 정보는 그대로 보존합니다. MOV 파일도 MP4 호환 구조일 때 처리할 수 있습니다. 다른 영상·소리 코덱, 자막 트랙, 분할 저장된 MP4와 AVI·MKV·WEBM 등 다른 컨테이너는 아직 지원하지 않습니다.
