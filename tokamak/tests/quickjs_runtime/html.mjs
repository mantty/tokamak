async function outcome(callback) {
  try { return await callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

export async function htmlContracts() {
  const result = {};
  result.selectorValidation = await Promise.all(["", "[", "p:no-such-pseudo", "p", Symbol("p")].map(selector => outcome(() => {
    new HTMLRewriter().on(selector, {});
    return true;
  })));
  result.handlerValidation = await Promise.all([1, null, undefined, "bad"].map(element => outcome(() => {
    new HTMLRewriter().on("p", { element });
    return true;
  })));
  result.asyncHandlers = await outcome(async () => {
    const events = [];
    const rewriter = new HTMLRewriter().on("p", {
      async element(element) {
        await new Promise(resolve => setTimeout(resolve, 1));
        events.push("element");
        element.setAttribute("data-ready", "yes");
        element.onEndTag(async tag => {
          await Promise.resolve();
          events.push("endTag");
          tag.before("!");
        });
      },
      async text(text) {
        await Promise.resolve();
        if (text.text) { events.push("text"); text.replace(text.text.toUpperCase()); }
      },
    }).onDocument({ async end(end) { await Promise.resolve(); end.append("done"); } });
    return { html: await rewriter.transform(new Response("<p>hello</p>")).text(), events };
  });
  result.rejection = await outcome(async () => {
    const failure = new Error("handler failed");
    const response = new HTMLRewriter().on("p", { async element() { await Promise.resolve(); throw failure; } }).transform(new Response("<p>hello</p>"));
    try { await response.text(); return "ignored"; }
    catch (error) { return { same: error === failure, message: error.message }; }
  });
  result.streaming = await outcome(async () => {
    let input;
    const source = new ReadableStream({ start(controller) { input = controller; controller.enqueue(new TextEncoder().encode("<p>first</p>")); } });
    const response = new HTMLRewriter().on("p", { element(element) { element.setAttribute("id", "ready"); } }).transform(new Response(source));
    const reader = response.body.getReader();
    const reading = reader.read();
    const first = await Promise.race([reading, new Promise(resolve => setTimeout(() => resolve(null), 200))]);
    input.enqueue(new TextEncoder().encode("<p>second</p>"));
    input.close();
    const chunks = [first ?? await reading];
    while (true) { const chunk = await reader.read(); if (chunk.done) break; chunks.push(chunk); }
    return { beforeEnd: first !== null, html: chunks.map(chunk => new TextDecoder().decode(chunk.value)).join("") };
  });
  result.expiredToken = await outcome(async () => {
    let held;
    await new HTMLRewriter().on("p", { element(element) { held = element; } }).transform(new Response("<p id=x>hello</p>")).text();
    return Promise.all([
      outcome(() => held.getAttribute("id")), outcome(() => held.tagName),
      outcome(() => { held.setAttribute("id", "late"); return true; }),
    ]);
  });
  result.attributesSnapshot = await outcome(async () => {
    let iterator;
    const fields = [];
    await new HTMLRewriter().on("p", { element(element) {
      iterator = element.attributes;
      element.setAttribute("Ä", "new");
      fields.push(element.getAttribute("ä"), element.getAttribute("Ä"));
      element.tagName = "SECTION";
      fields.push(element.tagName);
    } }).transform(new Response('<p Ä="value"></p>')).text();
    return { fields, next: await outcome(() => iterator.next()) };
  });
  result.modifiedAttributesIterator = await outcome(async () => {
    let next;
    await new HTMLRewriter().on("p", { async element(element) {
      const iterator = element.attributes;
      element.setAttribute("a", "new");
      next = await outcome(() => iterator.next());
    } }).transform(new Response('<p a="old"></p>')).text();
    return next;
  });
  result.transformStartsInput = await outcome(async () => {
    const input = new Response("<p>value</p>");
    let called = false;
    const output = new HTMLRewriter().on("p", { element() { called = true; } }).transform(input);
    const used = input.bodyUsed;
    await new Promise(resolve => setTimeout(resolve, 10));
    const started = called;
    await output.text();
    return { used, started };
  });
  result.multipleHandlers = await outcome(async () => {
    const removed = [];
    const output = new HTMLRewriter().on("p", { element(element) { element.remove(); } })
      .on("p", { element(element) { removed.push(element.removed); } })
      .transform(new Response("<p>value</p>"));
    return { html: await output.text(), removed };
  });
  result.streamReplacement = await outcome(async () => {
    const content = new ReadableStream({ start(controller) {
      controller.enqueue(new TextEncoder().encode("<b>"));
      controller.enqueue(new TextEncoder().encode("value</b>"));
      controller.close();
    } });
    return new HTMLRewriter().on("p", { element(element) {
      element.setInnerContent(content, { html: true });
    } }).transform(new Response("<p>original</p>")).text();
  });
  result.discardedReplacement = await outcome(async () => {
    let cancelled = false;
    let locked;
    const discarded = new ReadableStream({ cancel() { cancelled = true; return new Promise(() => {}); } });
    const response = new HTMLRewriter().on("p", { element(element) {
      element.setInnerContent(discarded);
      element.setInnerContent("done");
      locked = discarded.locked;
    } }).transform(new Response("<p>original</p>"));
    const html = await Promise.race([response.text(), new Promise(resolve => setTimeout(() => resolve("stuck"), 100))]);
    return { html, cancelled, locked };
  });
  result.unusualCancellation = await outcome(async () => {
    const reader = new HTMLRewriter().transform(new Response(new ReadableStream())).body.getReader();
    const pending = reader.read();
    const observed = outcome(() => pending);
    await reader.cancel(Object.create(null));
    return { pending: await observed, closed: await reader.closed.then(() => true) };
  });
  result.invalidReplacementCancelsSource = await outcome(async () => {
    let cancelled = 0;
    const replacement = new ReadableStream({
      start(controller) {
        controller.enqueue(new Uint8Array([0xc2]));
        controller.enqueue(new Uint8Array([0xa3, 0xff]));
      },
      cancel() { cancelled++; },
    });
    const response = new HTMLRewriter().on("p", { element(element) {
      element.setInnerContent(replacement);
    } }).transform(new Response("<p></p>"));
    const failure = await outcome(() => response.text());
    return { failure, cancelled };
  });
  result.endTagReplacement = await outcome(async () => {
    const calls = [];
    const html = await new HTMLRewriter().on("p", { element(element) {
      element.onEndTag(() => calls.push("old"));
      element.onEndTag(() => calls.push("new"));
    } }).transform(new Response("<p>text</p>")).text();
    return { calls, html };
  });
  result.caughtMutationErrors = await outcome(async () => {
    const errors = [];
    const input = "<p>text<!--comment--></p><br>";
    const response = new HTMLRewriter().on("*", {
      element(element) {
        for (const update of [() => element.setAttribute("bad name", "x"), () => { element.tagName = "1bad"; }]) {
          try { update(); errors.push("accepted"); } catch (error) { errors.push(error.name); }
        }
        if (element.tagName === "br") {
          try { element.onEndTag(() => {}); errors.push("accepted"); } catch (error) { errors.push(error.name); }
        }
      },
      comments(comment) {
        try { comment.text = "bad-->"; errors.push("accepted"); } catch (error) { errors.push(error.name); }
      },
    }).transform(new Response(input));
    return { html: await response.text(), errors };
  });
  result.charsets = await Promise.all(["windows-1252", '"ISO-8859-1"', "UTF-16", "not-an-encoding"].map(charset => outcome(async () => {
    const input = new Response(new Uint8Array([60,112,62,0xe9,60,47,112,62]), { headers: { "content-type": `text/html; charset=${charset}` } });
    const text = [];
    const output = new HTMLRewriter().on("p", { text(token) {
      if (token.text) { text.push(token.text); token.replace("£"); }
    } }).transform(input);
    const bytes = [...new Uint8Array(await output.arrayBuffer())];
    return { text, bytes, used: input.bodyUsed };
  })));
  result.thrownValues = await Promise.all([new TypeError("bad type"), new RangeError("bad range"), "oops", null].map(value => outcome(async () => {
    const response = new HTMLRewriter().on("p", { element() { throw value; } }).transform(new Response("<p></p>"));
    try { await response.text(); return "ignored"; }
    catch (error) { return { name: error?.name, message: error?.message, value: typeof error }; }
  })));
  result.reentrantHandler = await outcome(async () => new HTMLRewriter().on("p", { async element(element) {
    const nested = await new HTMLRewriter().on("b", { element(token) { token.tagName = "em"; } })
      .transform(new Response("<b>nested</b>")).text();
    element.setInnerContent(nested, { html: true });
  } }).transform(new Response("<p>original</p>")).text());
  result.cancelInput = await outcome(async () => {
    let cancelReason, cancelled = false;
    const source = new ReadableStream({ cancel(reason) { cancelled = true; cancelReason = reason; } });
    const reader = new HTMLRewriter().transform(new Response(source)).body.getReader();
    const pending = reader.read();
    const observed = outcome(() => pending);
    await reader.cancel("stop input");
    return { cancelled, locked: source.locked, reason: cancelReason, read: await observed, closed: await reader.closed.then(() => true) };
  });
  result.cancelHandler = await outcome(async () => {
    let entered, cancelReason, cancelled = false;
    const started = new Promise(resolve => { entered = resolve; });
    const source = new ReadableStream({
      start(controller) { controller.enqueue(new TextEncoder().encode("<p>text</p>")); },
      cancel(reason) { cancelled = true; cancelReason = reason?.message ?? reason; },
    });
    const reader = new HTMLRewriter().on("p", { async element() {
      entered();
      await new Promise(() => {});
    } }).transform(new Response(source)).body.getReader();
    const pending = reader.read();
    const observed = outcome(() => pending);
    await started;
    await reader.cancel("stop handler");
    return { cancelled, locked: source.locked, reason: cancelReason, read: await observed, closed: await reader.closed.then(() => true) };
  });
  return result;
}
