// Text helpers (rewritten as a class for clarity).
"use strict";

const SMALL = new Set(["a", "an", "the", "of", "in", "on", "and"]);

class Text {
  static capitalise(word) {
    return word.charAt(0).toUpperCase() + word.slice(1).toLowerCase();
  }

  static titleCase(input) {
    const words = input.split(" ");
    const out = [];
    for (let index = 0; index < words.length; index += 1) {
      const lower = words[index].toLowerCase();
      if (index > 0 && SMALL.has(lower)) {
        out.push(lower);
      } else {
        out.push(Text.capitalise(words[index]));
      }
    }
    return out.join(" ");
  }

  static slugify(input) {
    const lowered = input.toLowerCase();
    const dashed = lowered.replace(/[^a-z0-9]+/g, "-");
    return dashed.replace(/^-+|-+$/g, "");
  }

  static truncate(input, max) {
    if (input.length <= max) {
      return input;
    }
    return input.slice(0, max - 1).trimEnd() + "\u2026";
  }

  static wrap(input, width) {
    const lines = [];
    let current = "";
    for (const word of input.split(/\s+/)) {
      if (current && (current + " " + word).length > width) {
        lines.push(current);
        current = word;
      } else {
        current = current ? current + " " + word : word;
      }
    }
    if (current) {
      lines.push(current);
    }
    return lines;
  }

  static countWords(input) {
    const trimmed = input.trim();
    return trimmed ? trimmed.split(/\s+/).length : 0;
  }
}

module.exports = {
  titleCase: Text.titleCase,
  slugify: Text.slugify,
  truncate: Text.truncate,
  wrap: Text.wrap,
  countWords: Text.countWords,
};
