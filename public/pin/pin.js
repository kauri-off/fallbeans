// PIN entry: POST /fallbeans/api/auth sets a year-long cookie for /fallbeans/.
(() => {
  const form = document.getElementById('form');
  const inputs = [...document.querySelectorAll('#digits input')];
  const msg = document.getElementById('msg');
  const go = document.getElementById('go');
  const base = location.pathname.replace(/pin\/?.*$/, '');

  const pin = () => inputs.map((i) => i.value).join('');
  const fail = (text) => {
    msg.textContent = text;
    form.classList.remove('shake');
    void form.offsetWidth;
    form.classList.add('shake');
    for (const i of inputs) i.value = '';
    inputs[0].focus();
  };

  inputs.forEach((input, i) => {
    input.addEventListener('input', () => {
      // Several digits at once (fast typing, autofill): spread them over the following boxes.
      const digits = input.value.replace(/\D/g, '').split('');
      input.value = digits.shift() ?? '';
      let k = i + 1;
      for (; digits.length && k < inputs.length; k++) inputs[k].value = digits.shift();
      if (input.value) inputs[Math.min(k, inputs.length - 1)].focus();
      if (pin().length === inputs.length) form.requestSubmit();
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Backspace' && !input.value && i > 0) inputs[i - 1].focus();
    });
    input.addEventListener('paste', (e) => {
      const text = (e.clipboardData?.getData('text') ?? '').replace(/\D/g, '').slice(0, inputs.length);
      if (!text) return;
      e.preventDefault();
      text.split('').forEach((d, k) => {
        inputs[k].value = d;
      });
      if (text.length === inputs.length) form.requestSubmit();
    });
  });

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    if (pin().length !== inputs.length) return fail('Введите все цифры');
    go.disabled = true;
    msg.textContent = '';
    try {
      const r = await fetch(`${base}api/auth`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify({ pin: pin() }),
      });
      if (r.ok) {
        location.replace(base);
        return;
      }
      fail(
        r.status === 429
          ? 'Слишком много попыток — подождите минуту'
          : r.status === 401
            ? 'Неверный PIN-код'
            : 'Не удалось войти',
      );
    } catch {
      fail('Нет связи с сервером');
    } finally {
      go.disabled = false;
    }
  });

  inputs[0].focus();
})();
