// Syntax highlighting for code blocks, shared by the editor and the preview. Uses the same
// Lezer language parsers as the editor (@codemirror/language-data), loaded on demand.
import { LanguageDescription } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { highlightCode, tagHighlighter, tags as t } from '@lezer/highlight'

/** Maps syntax tags to `tok-*` classes, colored in styles.css (theme-aware). */
export const codeHighlighter = tagHighlighter([
  { tag: [t.keyword, t.controlKeyword, t.moduleKeyword, t.operatorKeyword, t.definitionKeyword, t.modifier, t.self], class: 'tok-keyword' },
  { tag: [t.string, t.special(t.string), t.regexp, t.character], class: 'tok-string' },
  { tag: [t.number, t.integer, t.float], class: 'tok-number' },
  { tag: [t.bool, t.null, t.atom, t.constant(t.variableName)], class: 'tok-atom' },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], class: 'tok-comment' },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName))], class: 'tok-function' },
  { tag: [t.definition(t.variableName), t.definition(t.propertyName)], class: 'tok-definition' },
  { tag: [t.typeName, t.className, t.namespace, t.standard(t.typeName)], class: 'tok-type' },
  { tag: t.propertyName, class: 'tok-property' },
  { tag: [t.operator, t.derefOperator, t.arithmeticOperator, t.logicOperator, t.compareOperator], class: 'tok-operator' },
  { tag: [t.punctuation, t.bracket, t.separator], class: 'tok-punct' },
  { tag: [t.meta, t.annotation, t.processingInstruction], class: 'tok-meta' },
  { tag: [t.tagName, t.angleBracket], class: 'tok-tag' },
  { tag: t.attributeName, class: 'tok-attr' },
  { tag: t.variableName, class: 'tok-variable' },
  { tag: [t.invalid], class: 'tok-invalid' },
])

/** Finds the language for a fence label like `python`, `py`, `ts`, `rust`. */
export function findLanguage(name: string) {
  return LanguageDescription.matchLanguageName(languages, name.trim(), true)
}

/** Replaces a <code> element's text with syntax-highlighted spans. */
export async function highlightElement(el: HTMLElement, lang: string): Promise<void> {
  const desc = findLanguage(lang)
  if (!desc) return
  const support = desc.support ?? (await desc.load())
  const code = el.textContent ?? ''
  const tree = support.language.parser.parse(code)
  const frag = document.createDocumentFragment()
  highlightCode(
    code,
    tree,
    codeHighlighter,
    (text, classes) => {
      if (!classes) return void frag.append(text)
      const span = document.createElement('span')
      span.className = classes
      span.textContent = text
      frag.append(span)
    },
    () => frag.append('\n'),
  )
  el.replaceChildren(frag)
}
