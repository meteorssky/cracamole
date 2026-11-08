// Protocol-specific advanced settings. One table drives how they are drawn, filled and saved,
// so a setting is declared once and only appears for the protocols that really use it.
import { $, el } from '../util/dom.js';
import { preserveSavedEncoding } from '../util/encodings.js';

import { OPTION_SECTIONS } from './option-sections.js';
export { OPTION_SECTIONS };

// A section lists the protocols it applies to, or 'all' for every protocol the server offers.
const appliesTo = (section, protocol) => section.protocols === 'all' || section.protocols.includes(protocol);

const inputId = name => 'opt-' + name;

function fieldNode(field) {
  const id = inputId(field.name);
  const hintId = 'hint-' + id;
  const errorId = 'error-' + field.name;
  const describedBy = [field.hint ? hintId : null, errorId].filter(Boolean).join(' ');
  if (field.type === 'checkbox') {
    return el('label', { class: 'check' }, el('input', { id, name: field.name, type: 'checkbox' }), el('span', {}, field.label));
  }
  const control =
    field.type === 'select'
      ? el('select', { class: 'input', id, name: field.name, 'aria-describedby': describedBy }, field.options.map(([value, label]) => el('option', { value }, label)))
      : el('input', {
          class: 'input', id, name: field.name, type: field.type === 'number' ? 'number' : 'text',
          min: field.min, max: field.max, maxlength: field.maxlength, placeholder: field.placeholder,
          autocomplete: 'off', spellcheck: field.type === 'text' ? 'false' : null, 'aria-describedby': describedBy,
        });
  return el(
    'div', { class: 'field' },
    el('label', { class: 'field__label', for: id }, field.label),
    control,
    field.hint ? el('p', { class: 'field__hint', id: hintId }, field.hint) : null,
    el('p', { class: 'field__error', id: errorId, hidden: true }),
  );
}

// Draw every section once; the editor shows only the ones whose data-protocols match.
export function renderOptions(container) {
  container.replaceChildren(
    ...OPTION_SECTIONS.map(section =>
      el(
        'details',
        { class: 'group group--details', 'data-protocols': section.protocols === 'all' ? null : section.protocols.join(' '), 'data-section': section.id },
        el(
          'summary',
          {},
          el('span', {}, section.title),
          el('span', { class: 'badge', 'data-summary-for': section.id }, '預設'),
        ),
        el(
          'div',
          { class: 'stack' },
          section.hint ? el('p', { class: 'field__hint' }, section.hint) : null,
          ...section.fields.map(fieldNode),
        ),
      ),
    ),
  );
}

const fieldsOf = section => section.fields;

function isFieldSet(form, field) {
  const control = form.elements[field.name];
  return field.type === 'checkbox' ? control.checked : Boolean(control.value.trim());
}

export function updateOptionSummaries(form, protocol) {
  let total = 0;
  let anyOpen = false;
  for (const section of OPTION_SECTIONS) {
    const count = fieldsOf(section).filter(field => isFieldSet(form, field)).length;
    const badge = form.querySelector('[data-summary-for="' + section.id + '"]');
    if (badge) {
      badge.textContent = count > 0 ? '已設定 ' + count + ' 項' : '預設';
      if (count > 0) badge.dataset.configured = 'true';
      else delete badge.dataset.configured;
    }
    if (appliesTo(section, protocol)) {
      total += count;
      const details = form.querySelector('[data-section="' + section.id + '"]');
      if (details?.open) anyOpen = true;
    }
  }
  $('options-summary').textContent = total > 0 ? '進階設定（已自訂 ' + total + ' 項）' : '進階設定';
  $('collapse-options').hidden = !anyOpen;
}

export function collapseAllOptions(form, protocol) {
  for (const details of form.querySelectorAll('.group--details')) details.open = false;
  updateOptionSummaries(form, protocol);
}

export function fillOptions(form, params, isEditing = false) {
  for (const section of OPTION_SECTIONS) {
    let hasValue = false;
    for (const field of fieldsOf(section)) {
      const control = form.elements[field.name];
      const value = params[field.name] || '';
      if (field.name === 'character-encoding') preserveSavedEncoding(control, value);
      if (field.type === 'checkbox') control.checked = value === 'true';
      else control.value = value;
      if (field.type === 'checkbox' ? control.checked : Boolean(value)) hasValue = true;
    }
    const details = form.querySelector('[data-section="' + section.id + '"]');
    if (details) {
      if (!isEditing) details.open = false;
      else if (hasValue) details.open = true;
    }
  }
}

// Returns [field, message] for the first invalid number among the options that apply, else null.
export function validateOptions(form, protocol) {
  for (const section of OPTION_SECTIONS) {
    if (!appliesTo(section, protocol)) continue;
    for (const field of fieldsOf(section)) {
      if (field.type !== 'number') continue;
      const text = form.elements[field.name].value.trim();
      if (!text) continue;
      const value = Number(text);
      if (!Number.isInteger(value) || value < field.min || value > field.max) {
        return [field.name, field.label + '須為 ' + field.min + ' 到 ' + field.max + ' 的整數，或留空使用預設值。'];
      }
    }
  }
  return null;
}

// Writes the options that apply to `protocol` into `params` (empty = default = no parameter),
// and removes every option that belongs only to other protocols.
export function readOptions(form, protocol, params) {
  for (const section of OPTION_SECTIONS) {
    const applies = appliesTo(section, protocol);
    for (const field of fieldsOf(section)) {
      const control = form.elements[field.name];
      const value = applies ? (field.type === 'checkbox' ? (control.checked ? 'true' : '') : control.value.trim()) : '';
      if (value) params[field.name] = value;
      else delete params[field.name];
    }
  }
}
