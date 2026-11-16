# -*- coding: utf-8 -*-
# PlanetX Lite - глобус PlanetX без QGIS.
# Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
# SPDX-License-Identifier: Apache-2.0
"""Сторож прозы - длинное тире, точка с запятой и стоп-слова.

    python -I tools/check_prose.py              проверить тексты Lite
    python -I tools/check_prose.py --self-test  проверить сам сторож

Правила - свод текстов линейки C:\\Dev\\workplace\\TEXTS.md. Проверяются
тексты для людей - строки интерфейса по-русски, README, заметки
выпусков, посты. Код в обратных кавычках, блоки кода и адреса
пропускаются, в коде точка с запятой допустима. Ритм и смысл сторож
не меряет, их проверяет чтение.

Выход 0 - находок нет, 1 - есть находки, каждая с файлом и строкой.
"""
import glob
import os
import re
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATTERNS = (
    "README.md",
    "doc/PLAN.md",
    "crates/ui/i18n/ru.tsv",
    "packaging/notes/*.md",
    "site/posts_*.md",
)

# Стоп-слова свода с корнями. Замена - в TEXTS.md, раздел «Слова».
STOP = re.compile(
    r"(?<![\w-])("
    r"честн\w*|нечестн\w*|вр[её]т|врать|врут|соврат\w*|"
    r"грабл\w*|софт\w*|кучк\w*|кучу|членени\w*|наблюдённ\w*|"
    r"мерк(?:а|и|у|е|ой|ам|ами|ах)|молча|руками|"
    r"предъяв\w*|соблазн\w*|лесенк\w*|вперемешку|под рукой|мелоч\w*|"
    r"simply|seamless\w*"
    r")(?![\w-])", re.IGNORECASE)
CODE = re.compile(r"`[^`\n]*`")
LIST = re.compile(r"\s*(?:[-*]|\d+[.)])\s")
URL = re.compile(r"https?://[^\s)\]>\"']+|&\w+;|&#\d+;")


def files(root=None):
    root = root or ROOT
    out = []
    for pattern in PATTERNS:
        out += sorted(glob.glob(os.path.join(root, pattern)))
    return out


def prose_lines(path):
    """Строки прозы файла: номер и текст без кода и адресов."""
    tsv = path.endswith(".tsv")
    fence = False
    in_list = False
    with open(path, encoding="utf-8") as fh:
        for n, line in enumerate(fh, 1):
            if line.lstrip().startswith("```"):
                fence = not fence
                continue
            if fence:
                continue
            if tsv:
                # Ключ\tперевод - проверяется только перевод.
                parts = line.rstrip("\n").split("\t", 1)
                if len(parts) < 2:
                    continue
                line = parts[1]
            else:
                # Точка с запятой в конце строки перечня - разметка
                # перечня, а не пунктуация прозы (свод, «Пунктуация»).
                if LIST.match(line):
                    in_list = True
                elif not line.strip() or not line.startswith(" "):
                    in_list = False
                if in_list:
                    line = re.sub(r";\s*$", "\n", line)
            yield n, URL.sub(" ", CODE.sub(" ", line))


def check(path, root=None):
    """Находки файла: «путь:строка вид - фрагмент»."""
    root = root or ROOT
    name = os.path.relpath(path, root).replace(os.sep, "/")
    found = []
    for n, text in prose_lines(path):
        if "—" in text:
            found.append("%s:%d длинное тире" % (name, n))
        if ";" in text:
            found.append("%s:%d точка с запятой" % (name, n))
        for m in STOP.finditer(text):
            found.append("%s:%d стоп-слово «%s»" % (name, n, m.group(1)))
    return found


def self_test():
    """Сторож обязан краснеть на заведомых нарушениях и молчать на коде."""
    planted = ("Строка с длинным тире — вот оно.\n"
               "Строка с точкой с запятой; вот она.\n"
               "Строка со стоп-словом, правится руками.\n"
               "Код `a; b` и адрес https://x.org/?a=1;b=2 не в счёт.\n"
               "- строка перечня с точкой с запятой в конце;\n"
               "  и её продолжение тоже;\n"
               "- последняя строка перечня.\n"
               "\n"
               "Обычный абзац в конце строки тоже ловится;\n"
               "```\nfn f(); // — в блоке кода не в счёт\n```\n")
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "README.md")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(planted)
        found = check(path, tmp)
    kinds = sorted(f.split(" ", 1)[1].split(" ")[0] for f in found)
    if kinds != ["длинное", "стоп-слово", "точка", "точка"]:
        print("сторож ошибается: %s" % found)
        return 1
    print("сторож краснеет на четырёх подложенных нарушениях, "
          "код и концы строк перечня пропускает")
    return 0


def main(argv):
    sys.stdout.reconfigure(encoding="utf-8")
    if "--self-test" in argv:
        return self_test()
    found = []
    checked = files()
    for path in checked:
        found += check(path)
    for f in found:
        print(f)
    print("файлов %d, находок %d" % (len(checked), len(found)))
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
