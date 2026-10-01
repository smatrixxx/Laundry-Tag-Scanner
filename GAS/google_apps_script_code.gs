// ==== Script Properties (Project Settings → Script Properties) ====
// GEMINI_API_KEY — настоящий ключ Gemini (aistudio.google.com/app/apikey),
//                  наружу НЕ уходит, используется только здесь, внутри GAS
// GEMINI_MODEL   — необязательно, по умолчанию gemini-1.5-flash
// PROXY_SECRET   — общий секрет между Rust-бэкендом и этим прокси, любая
//                  случайная строка

const PROPS = PropertiesService.getScriptProperties();

/**
 * Прокси в Gemini API для Rust-бэкенда (обход региональной блокировки:
 * сам GAS работает на инфраструктуре Google и её не видит).
 *
 * Ожидает POST-тело: {"secret": "...", "model": "...", "prompt": "...", "image": "<base64 без префикса data:...>"}
 * Отвечает: {"result": "текст"} либо {"error": "..."}
 */
function doPost(e) {
  try {
    if (!e || !e.postData) {
      return jsonResponse_({ error: 'no request body (запущено не как HTTP-запрос)' });
    }

    const body = JSON.parse(e.postData.contents);

    const expectedSecret = PROPS.getProperty('PROXY_SECRET');
    if (expectedSecret && body.secret !== expectedSecret) {
      return jsonResponse_({ error: 'forbidden' });
    }

    const apiKey = PROPS.getProperty('GEMINI_API_KEY');
    if (!apiKey) return jsonResponse_({ error: 'GEMINI_API_KEY не задан в Script Properties' });

    const image = body.image;
    if (!image) return jsonResponse_({ error: 'поле image обязательно' });

    const model = body.model || PROPS.getProperty('GEMINI_MODEL') || 'gemini-1.5-flash';
    const prompt = body.prompt || 'Что на этом изображении?';

    const url = 'https://generativelanguage.googleapis.com/v1beta/models/' + model + ':generateContent?key=' + apiKey;
    const payload = {
      contents: [
        {
          parts: [
            { text: prompt },
            { inline_data: { mime_type: 'image/jpeg', data: image } }
          ]
        }
      ]
    };

    const response = UrlFetchApp.fetch(url, {
      method: 'post',
      contentType: 'application/json',
      payload: JSON.stringify(payload),
      muteHttpExceptions: true
    });

    const json = JSON.parse(response.getContentText());
    const text = json && json.candidates && json.candidates[0] &&
      json.candidates[0].content && json.candidates[0].content.parts &&
      json.candidates[0].content.parts[0] && json.candidates[0].content.parts[0].text;

    if (!text) {
      return jsonResponse_({ error: 'пустой ответ от Gemini', raw: response.getContentText() });
    }

    return jsonResponse_({ result: text });
  } catch (err) {
    return jsonResponse_({ error: String(err) });
  }
}

function jsonResponse_(obj) {
  return ContentService.createTextOutput(JSON.stringify(obj)).setMimeType(ContentService.MimeType.JSON);
}
