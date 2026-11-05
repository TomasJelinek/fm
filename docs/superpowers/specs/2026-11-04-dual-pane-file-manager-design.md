# Dvoupanelový souborový manažer v Rustu

Datum: 4. listopadu 2026; doplněno 5. listopadu 2026

Stav: průběžně aktualizovaný návrh zahrnující všechny dosud přidané požadavky. Read-only základ, commander ovládání, automatická obnova, témata/fonty/nastavení, karty a uložené cesty, systémové ikony disků, pixelové rolování a plánování velikostí podle úložiště jsou implementované a otestované. Aktuální verze je0.1; zmenšený Touch profil má40px základní cíle. Viewer/VFS a opakované sudo relace mají samostatný návrh; kopírování/přesun/mazání s frontou a konflikty patří do dalších etap. Stav jednotlivých částí je v oddílu 18.

## 1. Záměr a potvrzená rozhodnutí

Vznikne svižný desktopový souborový manažer inspirovaný Altap Salamanderem a Total Commanderem. Začne malým, použitelným základem a bude se rozšiřovat po samostatně ověřitelných etapách.

Požadavky uživatele:

- Rust, UI ve Slintu, Tokio pro asynchronní orchestrace.
- Oddělený front-end a back-end. Uživatel zvolil **jednu aplikaci s oddělenými Rust crates a komunikací přes zprávy**.
- Dva nezávislé panely, zobrazení disků/připojených svazků a přepínání mezi nimi.
- Kopírování s povinným ověřením kontrolním součtem.
- Pomalé operace nesmějí blokovat ovládání aplikace.
- Windows x64 a Linux, zejména Ubuntu a Mint; macOS později.
- GPU využít tam, kde má měřitelný přínos.
- Možnost později využít C++ zdroje Salamanderu jako podklad pro chování a integraci s Windows.
- Dodatečně potvrzeno: bezpečnost má nejvyšší prioritu; všechny operace mimo prezentaci a vstup běží v back-endu, včetně úklidu velkých dat.
- Dodatečně potvrzeno: komplexní testy a **plná podpora Unicode v celém produktu**; to je zásadní rozdíl oproti referenci Salamanderu. Žádné ANSI převody názvů/cest. Textové Windows API musí používat varianty `W`; Unicode se svévolně nenormalizuje a zobrazení není identita souboru.
- Dodatečně potvrzeno: **žádný aplikační maximální počet znaků/bajtů cesty**, zejména žádné omezení na Windows `MAX_PATH`. Cesty mají dynamickou délku. Budoucí zdroje zahrnují také FTP a další vzdálené providery; jejich cesty nejsou lokální Windows cesty.
- Dodatečně potvrzeno: Windows, Linux a další platformní implementace jsou v oddělených souborech/crates a implementují společná rozhraní. Nepřidávat těla implementací pro několik OS do jednoho souboru.
- Dodatečně potvrzeno: veřejný Rust Style Guide a Rust API Guidelines, `rustfmt` s cílem 100 znaků na řádek, pevný limit ručně psaného zdrojového souboru **100 KB = 100 000 bajtů**. Podrobnosti a výjimky pro generovaný kód jsou v `CODE_STYLE.md`.
- Dodatečně potvrzeno: multiselekce, dávkové kopírování vybraného seznamu s průběhem a společná fronta kopírování/přesunu/mazání. Další operaci lze neblokujícím způsobem zadat během běhu jiné úlohy; zpracování nesmí zmrazit UI.
- Dodatečně potvrzeno: úplná spodní lišta F1–F10, příkazový editor se spouštěním výslovně zadaných příkazů v systémovém terminálu, Ctrl+[ / Ctrl+] vloží cestu levého/pravého panelu, Alt+Enter zobrazí vlastnosti, řádek `..`, systémové ikony a samostatná lišta disků/speciálních umístění nad každým panelem. Windows podporuje Shift+písmeno disku pouze při zaměření seznamu souborů.
- Dodatečně potvrzeno: Ctrl+Shift+S počítá velikosti všech složek aktivního panelu na pozadí; druhý stisk výpočet ruší. Aktivní panel je vizuálně zvýrazněný. Průběžné/neúplné velikosti se odlišují od dokončených výsledků. Odkazy se při výpočtu nepřecházejí; výsledek je živý součet logických délek, nikoli atomický snapshot.
- Uživatel pro tuto etapu zvolil lištu a ovládání nad read-only základem; kopírování/přesun/mazání a jejich fronta zůstávají další etapou. Podrobnosti nového ovládání a rozhraní desktopových služeb jsou v [doplňujícím návrhu](2026-11-04-commander-controls-design.md).
- Dodatečně potvrzeno: oba viditelné panely sledují změny souborového systému přes samostatné platformní rozhraní. Události se sloučí: obnova po 1 sekundě klidu, nejpozději po 3 sekundách souvislého proudu změn. Periodická kontrola po 15 sekundách zachytí chybějící notifikace, zejména síťových disků. Registrace watcherů, porovnání i obnova dat běží na backendu; výběr, kurzor, scroll a rozepsaný příkaz se zachovají.
- Dodatečně potvrzeno: odstranit horní řádek s názvem aplikace/About, aby neubíral místo panelům. Přidat uživatelské nastavení s výběrem původního modrého, neutrálního tmavého a světlého tématu. Nabídnout kompaktní rozestupy pro desktop/myš a větší rozestupy a dotykové cíle pro touchscreen. Odkaz About zůstane v menu; Options bude u příkazového řádku a v menu.
- Dodatečně potvrzeno: velká složka nemá zdržovat výsledky menších složek. Povolit omezené dávky souběžného počítání na prokazatelně lokálním SSD; na HDD, síťových discích a neznámém/nejistém úložišti pouze jednu aktivní I/O úlohu. Souběh musí být omezený a rozpočet sdílený mezi panely. Síťové úložiště se nepovažuje za SSD podle rychlého připojení nebo cache.
- Dodatečně potvrzeno: vestavěný viewer má mít prostý text, zvýrazněný zdrojový kód a vykreslený Markdown. Nejprve navrhnout rozhraní a zkontrolovat viewer/pluginy Salamanderu; F3 se aktivuje až se skutečnou implementací.
- Dodatečně odsouhlaseno: opakované sudo má využít systémové ověření a vhodnou otevřenou relaci, nikoli ukládat heslo v aplikaci. Automatické znovupoužití z editoru vyžaduje samostatný bezpečný návrh relací z oddílu 24; samotné otevřené root okno tento routing nezajišťuje.
- Dodatečně potvrzeno: obecné VFS rozhraní má umožnit lokální disky, procházení archivů včetně vnořených, další vzdálené filesystémy a Windows Registry. Současné lokální panely se převedou na první nativní provider; Windows/Linux/macOS mají každý svou samostatnou nativní implementaci.
- Dodatečně potvrzeno: každý viewer a filesystem včetně nativního je dynamicky načitatelný plugin vedle executable ve složkách viewers/filesystems. Přítomnost balíčku určuje dostupnost funkce. První viewer je text se syntax highlighting; první archivy ZIP, GZIP, TAR.GZ a 7z. Rozhraní a přejímku finalizuje [plugin návrh](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).
- Dodatečně potvrzeno: při kopírování/přesunu na existující cíl zobrazit varování a rozhodnutí o přepsání se srovnáním zdroje/cíle, zejména velikosti a data/času. Bez výslovné platné politiky se nic nepřepíše.
- Dodatečně potvrzeno: výběr shellu vlevo od příkazového editoru na všech platformách: PowerShell/cmd na Windows, Terminal/Terminal (sudo) na Linuxu a stejné připravené rozhraní pro budoucí macOS. Enter při zaměřeném editoru spouští přesně jeho obsah ve zvolené externí relaci; volba se zachytí společně s cwd a oprávněním.

Ostatní konkrétní volby v tomto dokumentu jsou doporučení autora návrhu, nikoli další potvrzené požadavky uživatele. Zejména jde o BLAKE3, pořadí etap, počáteční rozsah metadat a navržené výkonnostní cíle.

## 2. Zvažované přístupy

| Přístup | Výhody | Náklady | Rozhodnutí |
| --- | --- | --- | --- |
| Jeden proces, oddělené crates, zprávy | Nízká režie, jednoduché balení, nezávisle testovatelný back-end | Pád procesu zasáhne celou aplikaci; blokující systémové volání nelze obecně násilně ukončit | Zvoleno uživatelem |
| Samostatný proces pro UI a back-end | Izolace pádů, možnost ukončit pracovní proces | IPC, přenos dat, verzování protokolu, složitější životní cyklus | Odloženo |

Vybraná architektura umožní později přesunout konkrétní rizikovou integraci do pomocného procesu. Nevyžaduje kvůli tomu dnes serializovat veškerou interní komunikaci ani zavádět síťové API.

## 3. Co přesně znamená neblokující aplikace

UI vlákno zpracovává vstup, kreslení a krátké aktualizace modelu. Neprovádí diskové operace, dotazy na síťové cesty, získávání ikon z OS, hashování ani řazení velkých seznamů. Ani uvolňování rozsáhlých snapshotů/indexů nesmí skrytě běžet v callbacku UI; taková data se předají omezené back-endové frontě k uvolnění. Nečeká na back-end pomocí `block_on`, `blocking_send`, `join` nebo dlouho drženého zámku.

U běžného souborového I/O Tokio používá blokující operace na pracovních vláknech. `async fn` tedy samo o sobě nezaručuje neblokující systémové I/O. Návrh zaručuje oddělení této práce od UI a omezení spotřeby prostředků; neslibuje okamžité dokončení ani okamžité zastavení libovolného volání OS. [Tokio: filesystem](https://docs.rs/tokio/latest/tokio/fs/index.html)

Práce bude rozdělena následovně:

| Oblast | Umístění | Pravidlo |
| --- | --- | --- |
| UI a Slint modely | Hlavní vlákno | Pouze krátká práce s daty v paměti |
| Fronty, plánování, časovače, koordinace úloh | Vícevláknový Tokio runtime | Bez dlouhých CPU výpočtů a blokujících volání v běžných async tasks |
| Blokující OS a souborové operace | Omezené pracovní kapacity | Omezení souběhu před spuštěním práce, sledování i po požadavku na zrušení |
| Velké řazení, hashování, budoucí dekódování náhledů | Omezená CPU/pracovní kapacita | Kopírování a hashování lze zpočátku sloučit do jednoho streamujícího workeru |

Krátké výpočty typu změny kurzoru zůstanou synchronní. Veškeré potenciálně pomalé operace mají asynchronní rozhraní vůči UI.

## 4. Rozdělení projektu

Cargo workspace rozděluje pět logických vrstev do následujících crates; platformní vrstva má oddělené implementace:

| Crate | Odpovědnost | Závislosti uvnitř workspace |
| --- | --- | --- |
| `fm-core` | Doménové typy, identifikátory, příkazy/události, chyby, rozhraní providerů a jejich schopnosti | Žádné |
| `fm-engine` | Navigace, načítání adresářů, plánování, kopírování, ověřování a stav úloh; Tokio | `fm-core` |
| `fm-platform` | Malá továrna/reexport podle targetu | `fm-core` a zvolený platformní crate |
| `fm-platform-common` | Přenositelné bezpečné Rust operace a společné pomocné struktury | `fm-core` |
| `fm-platform-windows` | Windows implementace služeb, Win32, svazky a Windows specifika | `fm-core`, `fm-platform-common` |
| `fm-platform-linux` | Linux implementace služeb, mountinfo a Linux specifika | `fm-core`, `fm-platform-common` |
| `fm-ui` | `.slint` soubory, klávesové ovládání, presenter, modely a převod dat pro zobrazení | `fm-core` |
| `fm-app` | Spuštění, konfigurace, vytvoření runtime, propojení zpráv, injektování providerů a ukončení | Ostatní crates |

`fm-core`, `fm-engine` a `fm-platform` neznají Slint. `fm-ui` nevolá filesystem ani konkrétní engine a není závislé na jeho implementaci. `fm-app` dodá presenteru neblokující vstupní rozhraní a propojí výstupní události s UI. Rozhraní providerů budou odpovídat potřebám aktuálních etap; nevznikne předem rozsáhlá abstrakce všech souborových systémů.

Pro lokální cesty se používá dynamický `PathBuf`/`OsString`, nikoli cesta rekonstruovaná z textu v UI nebo buffer o velikosti `MAX_PATH`. `PathBuf` je lokální reprezentace, nikoli univerzální formát pro FTP či jiné providery. Zobrazený název a identita položky jsou oddělené. Rust cesty nemusejí být platné UTF-8. [Rust: Path](https://doc.rust-lang.org/rust-by-example/std_misc/path.html)

Základní identifikátory: `PaneId`, `NavigationId`, `EntryId`, `JobId`, `VolumeId`. `EntryId` identifikuje položku v konkrétním výpisu; OS identita souboru je samostatná informace. Výběr ani příkazy se neodvozují jen z indexu řádku, který se může změnit řazením.

## 5. UI a komunikace s back-endem

Slint event loop i komponenty žijí na hlavním vlákně. Samostatně vytvořený Tokio runtime obsluhuje back-end. Výsledky se doručují přes bridge; změny Slint modelů se aplikují výhradně v UI vlákně. [Slint: threading](https://docs.slint.dev/latest/docs/rust/slint/#threading-and-event-loop)

Počáteční smlouva obsahuje zejména:

- Příkazy: `ListVolumes`, `Navigate`, `Refresh`, `StartCopy`, `CancelJob`, `ResolveConflict`.
- Události: `VolumesUpdated`, `ListingStarted`, `EntriesBatch`, `ListingFinished`, `ListingFailed`, `JobStateChanged`, `ConflictRaised`.
- Poslední průběžný stav: počet zpracovaných bajtů, aktuální soubor a fáze úlohy.

Jde o návrh významu zpráv, nikoli hotové Rust API. Přesné signatury a reprezentace async rozhraní jsou implementační volba.

Komunikace má tři režimy:

1. **Příkazy a rozhodnutí:** omezená fronta; callback UI nikdy nečeká na její uvolnění. Při zaplnění se příkaz nepovažuje za přijatý a UI zobrazí odpovídající stav. Požadavek na zrušení má samostatnou neblokující signalizaci.
2. **Výpisy adresářů:** omezené dávky a omezená fronta. Producent čeká v back-endu, pokud UI nestíhá. Každá dávka obsahuje panel a generaci navigace.
3. **Průběh úloh:** uchovává se poslední hodnota; není nutné doručovat každý mezikrok. Změny terminálních stavů a požadavky na rozhodnutí jsou spolehlivé, bez tichého zahazování.

Engine vlastní autoritativní registr úloh. Zavření zobrazení průběhu úlohu neztratí. UI může načíst aktuální stav znovu.

Omezení front platí až k UI: bridge nesmí vyprázdnit omezený kanál do neomezeného množství `invoke_from_event_loop` callbacků. Drží nejvýše jedno čekající probuzení; UI spotřebuje práci v časovém rozpočtu a případně naplánuje další průchod. Počty zpráv a běžné dávky položek jsou omezené; velké snapshoty se předávají sdíleným neměnným vlastnictvím. Rozpočet paměti nebo transportní rámec nesmí zavést skrytý maximální počet znaků cesty: dlouhá identita se přenáší sdíleně/tokenem či streamem a nedostatek prostředků vrací explicitní chybu bez zkrácení. [Tokio: mpsc](https://docs.rs/tokio/latest/tokio/sync/mpsc/), [Slint: invoke_from_event_loop](https://docs.slint.dev/latest/docs/rust/slint/fn.invoke_from_event_loop)

Při rychlém přepnutí A → B → C se starší výpisy zruší kooperativně a jejich opožděné dávky se podle `NavigationId` ignorují. Stejnou generaci nesdílí oba panely.

## 6. První podoba dvou panelů

Oba panely mají stejnou komponentu a vlastní stav: umístění, historii, řazení, kurzor, výběr, generaci navigace a stav načítání/chyby.

Každý panel obsahuje:

- Výběr disku/připojeného svazku a editovatelnou cestu.
- Seznam se sloupci název, velikost, datum změny a typ; adresáře jsou standardně první.
- Odlišený kurzor, výběr a aktivní panel; tyto stavy se neslévají.
- Stavový řádek s počtem položek a vybraných souborů; volné místo se načítá nezávisle.

Základní ovládání: Tab mezi panely, Enter nebo levý dvojklik otevře adresář v panelu či soubor ve výchozí systémové aplikaci, Backspace nadřazený adresář, šipky a Page Up/Down pohyb, Space/Insert výběr, F5 kopírování do druhého panelu po jeho zavedení. Levé kliknutí nastaví kurzor i aktivní panel; Ctrl+klik přepíná multivýběr. Zkratky budou definované jako akce, aby šly později přemapovat. Původní rozložení klávesových zkratek pro konkrétní OS ověříme v UI etapě.

Myší se aktivuje pouze levý dvojklik bez modifikátorů. Výběr, pravé kliknutí,
hover ani automatický refresh soubory nespouštějí. Token řádku zachycuje aktuální
navigační záměr, generaci skutečně zobrazeného výpisu a identitu položky; změna
řazení nebo výpisu mezi stisky nesmí otevřít náhradní položku. Aktualizace kurzoru
mění jen dotčené řádky, protože reset modelu by zrušil Slint detekci dvojkliku.

Nové `OpenService` klasifikuje explicitně otevřený odkaz na backendu: adresář
naviguje uvnitř panelu, běžný soubor použije systémovou asociaci, speciální nebo
chybějící soubor vrátí chybu s původní cestou. Windows má samostatnou implementaci
`ShellExecuteExW(open)` s dynamickými UTF-16 cestami a oddělenými parametry; Linux
použije absolutní systémový `xdg-open` nebo `gio open` s doslovným nativním
argumentem bez shellu. Metadata, spuštění i sklizeň pomocných procesů běží mimo
UI, s vlastním limitem čtyř volání a šestnácti sledovaných procesů. Starý výsledek
odkazu nesmí přepsat novější navigaci ani ukrást zaměření druhému panelu. Nejde
o garanci neměnné inode identity proti externím souběžným změnám souborového
systému; budoucí zápisy potřebují silnější smlouvu. Podrobnosti a referenční
kotvy Salamanderu jsou v `2026-11-04-mouse-activation-design.md`.

Aktuální rozšíření ovládání je podrobně sjednocené v oddílech 18–21: celé F1–F10,
příkazový editor, vlastnosti, lišty umístění, počítání velikostí, řazení, watchery,
témata a hustota. Horní branding řádek se nezobrazuje. Aktivní panel odlišuje
barevné záhlaví a jemný rámeček; kurzor a multivýběr zůstávají samostatné stavy.

Seznam bude virtualizovaný: pro viditelné řádky se vytvářejí UI prvky, nikoli samostatná komponenta pro každý soubor. Návrh počítá s `ListView`, vlastními řádky a společnou hlavičkou sloupců. Model vrací pouze data již v paměti, nikdy neprovádí I/O. Virtualizace sama neomezuje paměť potřebnou pro uložené položky ani cenu řazení. [Slint: ListView](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/views/listview/)

Jména se zobrazují postupně v dávkách. Dražší metadata se doplňují odděleně; neznámá velikost se nezobrazuje jako nula. Řazení velkých výpisů probíhá mimo UI a výsledky se aplikují s kontrolou generace. Aktualizace průběhu kopírování neobnovuje model celého adresáře. Velikost adresářů se při pouhém otevření panelu rekurzivně nepočítá.

## 7. Disky a rozdíly platforem

Společný `VolumeService` vrací seznam navigačních kořenů a jejich stav. UI nebude předpokládat, že svazek má písmeno disku.

- **Windows:** písmena disků a kořeny svazků přes Win32 adaptéry; názvy, volné místo a dostupnost se zjišťují odděleně. Později historie UNC cest a další integrace. [Microsoft: GetLogicalDriveStringsW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getlogicaldrivestringsw)
- **Linux:** kořen `/`, domovský adresář jako navigační zkratka a vhodně filtrované připojené filesystemy. Zdroj informací o mount namespace je `/proc/self/mountinfo`; interní pseudofilesystemy se standardně nezobrazují. V první etapě se přepíná mezi již připojenými svazky; připojování a bezpečné odpojování zařízení je samostatná pozdější funkce. [Linux kernel: mountinfo](https://www.kernel.org/doc/html/latest/filesystems/proc.html)
- **macOS později:** nový adaptér pro svazky, přístupová oprávnění a distribuci; doménový engine zůstává společný.

Nedostupný svazek má vlastní stav chyby. Na dotaz na jeho kapacitu se nečeká před vykreslením ostatních svazků. Vyčerpání kapacity kopírovacích workerů nesmí zablokovat veškerou kapacitu pro navigaci. Trvale zablokované systémové volání však může držet worker; návrh omezuje počet takových pokusů a nepřidává neomezeně další vlákna.

Nativní notifikace změn budou optimalizace. Při chybě, ztrátě událostí či návratu do panelu se použije nový výpis. Watcher není jediný zdroj pravdy, zejména na síťových filesystémech. [notify: known problems](https://docs.rs/notify/latest/notify/#known-problems)

## 8. Kopírování s povinným ověřením

### Algoritmus a rozsah záruky

Výchozí návrh je **BLAKE3 s 256bitovým výstupem**, průběžně počítaný nad bloky dat. Oficiální Rust implementace nabízí inkrementální hashování a optimalizace pro CPU. SHA-256 lze přidat pro interoperabilitu. Výběr BLAKE3 neznamená tvrzení, že bude na každém stroji rychlejší než hardwarově akcelerované SHA-256. [BLAKE3: Rust API](https://docs.rs/blake3/latest/blake3/)

Úspěšné kopírování běžného souboru znamená:

> Bajtový proud přečtený ze zdrojového handle a samostatný úplný čtecí průchod cílovým souborem mají shodnou délku a BLAKE3 hash. Nebyla zjištěna změna zdroje. Proběhlo požadované dokončení zápisu, synchronizace a publikování cíle.

Hash ověřuje běžný datový obsah. Zachování práv, ACL, NTFS alternate data streams, rozšířených atributů, hardlinků a sparse rozložení představuje samostatné schopnosti a není součástí prvního slibu bezeztrátové kopie. Kopírovací UI musí předem uvádět dostupnou politiku metadat; selhání slíbeného zachování metadat se nesmí vydávat za úplný úspěch.

### Postup pro jeden běžný soubor

1. Otevřít zdroj jednou a zaznamenat identitu a dostupné indikátory změny. Odmítnout kopii do téhož souboru, včetně detekovatelných aliasů.
2. Výhradně vytvořit vlastní dočasný soubor vedle cíle na stejném filesystemu. Název není předvídatelný a nesmí přepsat cizí soubor. Práce s handle a ověřování identity omezí závody při opakovaném vyhodnocování cest.
3. Po omezených blocích číst zdroj, počítat jeho hash a zapisovat všechna přečtená data. Nepoužívat načtení celého souboru do RAM. Velikost bloků bude měřená implementační volba.
4. Dokončit čekající zápisy, aplikovat slíbená metadata a provést synchronizaci souboru. `flush()` Tokia a synchronizace úložiště jsou odlišné kroky; chyba kteréhokoli povinného kroku je chyba úlohy.
5. Novým úplným čtecím průchodem ověřit dočasný cíl: vlastní handle se převine, případně se bezpečně znovu otevře se shodnou identitou. Porovnat délku a hash. Opětovné zahashování odesílaného bufferu není ověření cíle.
6. Zkontrolovat dostupné indikátory změny zdroje. Při zjištěné změně soubor neoznačit jako úspěšně zkopírovaný.
7. Publikovat ověřený soubor pod finálním názvem podle politiky kolize, pomocí platformního adaptéru. Původní cíl se předem nemaže.
8. Dokončit podporované kroky synchronizace adresářového záznamu. Až potom oznámit slíbený úspěch. Pokud se publikování povedlo, ale následná synchronizace selhala, výsledek výslovně uvádí „cíl publikován, trvalost nepotvrzena“.

Základní politika při kolizi je **přeskočit nebo zvolit jiný název**. Publikování bez přepsání musí řešit souběžný vznik cíle atomicky, nikoli kontrolou existence před běžným rename. Ověřené přepisování existujícího cíle se přidá po otestování platformních pravidel. Při nepodporované záruce se operace odmítne s vysvětlením; nepřejde tiše na smazání původního cíle.

Stejný filesystem umožní používat vhodné rename/publish operace; jejich podmínky a chování při selhání nejsou mezi Windows a Linuxem totožné. Platformní adaptér deklaruje podporované schopnosti. Referenční testování začíná lokálními NTFS a ext4, další filesystemy a síťové servery rozšíří ověřenou matici. Nelze obecně tvrdit, že libovolné přepsání je bezpečná transakce. [Rust: rename](https://doc.rust-lang.org/std/fs/fn.rename.html), [Microsoft: ReplaceFileW a jeho chybové stavy](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)

### Omezení, která mají význam pro správnost

- Nové přečtení cíle může být obsloužené cache OS. Ověření obsahu a odolnost při výpadku napájení jsou odlišné vlastnosti. Ani hash se synchronizací nepředstavuje absolutní záruku proti všem poruchám hardwaru. [Microsoft: file caching](https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching)
- Kontrola velikosti a času změny zachytí běžné změny zdroje, ale neprokazuje konzistentní snapshot během souběžného zápisu. Silnější režim později potřebuje vyloučení zapisovačů nebo snapshot daného filesystemu.
- Základní ověřování znamená jeden čtecí průchod zdrojem, zápis cíle a další čtecí průchod cílem. Jeho cenu měříme jako součást celkové rychlosti. Odezva UI zůstává nezávislá na době operace.
- Uživatelský panel s průběhem rozlišuje „kopírování“, „synchronizace“ a „ověřování“. Zapsání posledního bajtu neznamená hotovo.

### Omezený souběh podle skutečného úložiště

Pro budoucí kopírování použít nejprve souběh **různých souborů**, každý s vlastním
dočasným cílem, sekvenčním datovým proudem a samostatným úplným ověřením. Návrhový
výchozí limit je **2 aktivní I/O proudy na prokazatelně lokálním SSD**; nastavitelný
strop 4 se povolí pouze po měření celého přenosu včetně synchronizace a readbacku.
HDD, síť a neznámé úložiště začínají jedním proudem ve sdíleném rozpočtu příslušného
zařízení či konzervativní skupiny. SSD na vzdáleném serveru není lokální SSD.
Jde o konzervativní návrhové hodnoty, nikoli doporučení výrobců nebo garanci zrychlení.
Microsoft podporuje paralelní kopie v Robocopy; upozorňuje také, že příliš mnoho
vláken může výkon zhoršit. [Robocopy](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy),
[Microsoft: malé soubory a souběh](https://learn.microsoft.com/en-us/troubleshoot/windows-server/networking/slow-smb-file-transfer#slow-transfer-when-using-small-files)
Také desktopová měření výrobce SSD rozlišují queue depth a skutečnou odezvu;
maximální benchmarková propustnost neurčuje vhodný souběh této aplikace.
[Solidigm: PC Storage Performance in the Real World](https://www.solidigmtechnology.com/content/dam/solidigm/en/site/products/technology/performance-with-impact/documents/Performance-With-Impact-White-Paper-R2.pdf)

Počet vláken není počet I/O proudů ani hardwarová queue depth. Tokio koordinuje
omezené úlohy na backendu; skutečně blokující I/O běží v omezené pracovní kapacitě.
Zdrojové čtení, cílový zápis, synchronizace a ověřovací čtení sdílejí rozpočet
skutečného fyzického úložiště napříč panely a operacemi. Aliasy a oddíly C:/D: na
témže SSD nedostanou dva nezávislé rozpočty. Každé probíhající I/O se účtuje;
sekvenční fáze jednoho souboru na stejném zařízení mohou využít jeden permit,
souběžné čtení/zápis by vyžadovalo další. Neznámá topologie volí sdílený
konzervativní režim. Permit zůstává obsazen do skutečného návratu OS volání i po
zrušení. Přednost navigace, omezené fronty, bajty bufferů a CPU hashování jsou
samostatné podmínky přijetí práce, nikoli jen limit počtu tasks.
Prokázaná různá fyzická zařízení mohou překrývat fáze různých souborů. Velké
soubory po omezených úsecích a skutečném dokončení I/O uvolní pracovní permit
další připravené práci; jejich handle/offset/hash přitom zůstávají vlastněné a
omezené. Před zapnutím kopírování musí stejný rozpočet zařízení zahrnovat také
skeny velikostí a interaktivní čtení, aby se nezávislé limity nesčítaly.

Paralelní bloky jednoho velkého souboru jsou odložená, samostatně měřená
optimalizace. Zachovají přesné pořadí bajtů BLAKE3, omezené přeuspořádání a původní
publikační kontrakt; hashování bloků v pořadí dokončení nebo slepení jejich hashů
není hash původního souboru. Stávající čtyřslotový vážený scheduler velikostí
složek není hotový kopírovací algoritmus. Podrobnosti rozpočtů, férovosti,
publikování a testů určuje [návrh souběhu kopírování](2026-11-04-copy-concurrency-design.md).
**Toto je pouze návrh; kopírování/přesun/mazání zůstávají neimplementované a vypnuté.**

## 9. Stav úloh, rušení a chyby

Základní fáze: `Queued → Preparing → Copying → Syncing → Verifying → Publishing → Completed`. Vedlejší stavy: `WaitingForDecision`, `CancelRequested`, `Cancelled`, `Failed` a výsledek po publikování s nepotvrzenou trvalostí.

Engine vlastní všechny pracovní handles a sleduje i práci, jejíž UI už požádalo o zrušení. Rušení je kooperativní mezi bloky a fázemi. Již spuštěné `spawn_blocking` nelze zastavit pouhým `abort()`. Dočasný soubor se uklízí až po dokončení všech probíhajících operací nad ním. [Tokio: spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

Před publikováním zrušení zabrání publikování a následně odstraní vlastní dočasný soubor. Pokud už publikování začalo, vyčká se na jeho skutečný výsledek; úspěšně publikovaný soubor se kvůli souběžnému stisku Zrušit nemaže. Stav v UI v mezidobí odpovídá „ruší se“, nikoli předčasnému „zrušeno“.

U adresářové úlohy už dokončené soubory zůstávají. Celá složka není jedna atomická transakce. Chyby jednotlivých souborů mají konkrétní výsledek a souhrn úlohy rozlišuje úplný úspěch, částečné dokončení a zrušení. Dialog rozhodnutí pozastaví danou úlohu, nikoli UI nebo vlákno čekající na odpověď.

Po pádu mohou zůstat dočasné soubory. Bezpečně se evidují jako artefakty konkrétní úlohy; neodstraňují se libovolné soubory podle přípony. Automatické pokračování v rozkopírovaném souboru není součást prvního kopírovacího milníku.

Fronta, paměť a souběh jsou omezené. Velké kopie nesmějí vyhladovět interaktivní
navigaci. První kopírovací milník má zahrnout konzervativní rozpočty podle
zdrojového/cílového úložiště z oddílu 8 a oddělené pracovní kapacity pro
interaktivní práci. Jeden task nebo nové vlákno pro každý z milionu souborů je
nepřípustný; čekající požadavky se připravují postupně v omezeném okně.

## 10. GPU a měření výkonu

GPU použijeme pro vykreslování Slintu. Počáteční kandidát je Winit + FemtoVG-WGPU; proti Skia se ověří startup, plynulost, paměť a velikost distribuce. Zůstane explicitně testovaná softwarová varianta pro prostředí bez použitelné GPU. Winit pokrývá Windows, Linux X11/Wayland a macOS; Slint nabízí více GPU i softwarových rendererů. [Slint: Winit](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/), [Slint: renderery](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/)

Kopírování a hashování začnou na CPU. GPU hashování má přenosovou a synchronizační režii; jeho případné zavedení vyžaduje měřitelný přínos celému přenosu s ověřením. Vícevláknové hashování není automaticky zapnuté jen instalací BLAKE3; nesmí přesytit stroj ani zhoršit přístup k HDD.

Navržené cíle, které se musí potvrdit na zaznamenaném referenčním stroji:

- Při běžném ovládání cílit na 60 fps; aplikační aktualizace UI dávkovat přibližně do 2–4 ms na průchod.
- Při současném kopírování a výpisu 100 000 položek cílit na p95 odezvy vstupu do 50 ms; sledovat i p99 a nejdelší zásek.
- U lokálního zahřátého adresáře cílit na první použitelné položky do 100 ms. U studeného či nedostupného média se měří zvlášť a UI ihned zobrazuje načítání.
- Průběh kopírování obnovovat přibližně 10–20krát za sekundu; za nečinnosti zbytečně nepřekreslovat.
- Celková velikost kopírovaného souboru nesmí určovat velikost bufferu v RAM. Seznam adresáře může růst s počtem položek; paměť se měří odděleně od počtu viditelných řádků.

Výsledky vždy uvádějí OS, CPU, médium, filesystem, renderer, počet položek a stav cache. Měří se čas do prvních položek i úplného načtení, UI latency, paměť, zatížení CPU, obsazení front a celková rychlost kopie včetně synchronizace a ověření. Diagnostika začíná strukturovanými událostmi přes `tracing`, bez zaznamenávání obsahu souborů.

## 11. Etapy a jejich přejímací podmínky

### Etapa 1: skutečné dva panely a svazky

Vytvořit workspace, zprávový bridge a dva panely nad skutečným lokálním filesystemem. Zahrnout přepínání dostupných svazků, vstup cesty, historii, základní klávesové ovládání, postupné výpisy, řazení a stavy načítání/chyby. Ve výchozí podobě použít jednoduchý vzhled s jasným označením aktivního panelu.

Hotovo znamená: každý panel nezávisle naviguje, rychlé přepínání nevrací staré výsledky, pomalý dotaz na disk neblokuje UI a velký výpis vytváří pouze viditelné řádky. Windows a Linux mají průběžné sestavení a automatické kontroly od této etapy; testuje se alespoň jedna Ubuntu a jedna Mint instalace. Konkrétní minimální verze OS a distribuční formát se zvolí před balením, nikoli odhadem v doménovém jádru.

### Etapa 2: jeden nebo více běžných souborů s ověřením

Přidat F5, cílový panel, jednoduchou frontu, průběh fází, BLAKE3, dočasný cíl, úplný kontrolní čtecí průchod, synchronizaci, publikování bez přepsání a zrušení. Při kolizi přeskočit nebo vybrat jiné jméno. Zachovávat obsah a deklarovaná základní metadata; další typy položek zatím výslovně odmítnout.

Fronta od začátku používá společný model operací z oddílu 17. Dávka představuje
zachycený výběr více položek, nikoli jednu synchronní smyčku spuštěnou z UI.
Přesun/mazání se připojí ke stejné infrastruktuře po zavedení jejich bezpečných
platformních služeb; nebudou mít vlastní neslučitelné fronty.

Hotovo znamená: úspěch vzniká až po ověření; vložené poškození cíle je detekováno; existující cílový soubor se nezničí při chybě ani závodu; zrušení počká na doběhnutí workeru před úklidem. UI jde používat během celé operace.

### Etapa 3: adresáře, kolize a odolnost

Přidat postupný průchod stromem, multivýběr, rozhodování o konfliktech, ověřené přepisování na podporovaných filesystémech a souhrn jednotlivých výsledků. Výchozí politika pro symlinky/junctions je nenásledovat a uvést přeskočenou položku; zachování samotných odkazů se přidává jako explicitní schopnost. Běžné adresáře se kopírují bez čekání na úplný předběžný součet; do jeho zjištění je průběh neurčitý.

Zakázat kopii adresáře do sebe nebo vlastního podstromu, včetně zjistitelných aliasů; symlinky nesmějí způsobit smyčku. Existující watcher rozšířit o koordinaci s vlastními mutacemi a evidenci nedokončených artefaktů.

Hotovo znamená: otestované plné médium, odpojení disku, přístupová chyba, souběžná změna zdroje a cíle, zrušení v jednotlivých fázích a částečné dokončení stromu. Zachované zdrojové a cílové soubory odpovídají oznámenému výsledku; nepodporovaná metadata nejsou slibována.

### Etapa 4: další funkce po jedné

Na stejný model příkazů a úloh navázat přesun, přejmenování, vytváření složek, koš/mazání, hledání, záložky panelů, prohlížeč a náhledy. Přesun mezi filesystémy smí smazat zdroj až po úspěšné ověřené kopii a publikování. Přesun v rámci jednoho filesystemu je jiný typ operace s vlastními pravidly.

Archivy a vzdálené zdroje přidávat jako providery s deklarovanými schopnostmi: čtení, zápis, přejmenování, kontrolní čtení, publikování a politika metadat. Provider bez potřebné schopnosti nesmí předstírat stejnou záruku jako lokální ověřená kopie. Samostatné plugin API nebylo součástí původního základu; nový požadavek z 5. listopadu je zavádí jako podmínku viewer/VFS etapy, viz oddíly 22–23.

Počítá se výslovně s FTP/FTPS a případně SFTP; FTP, FTPS a SFTP jsou odlišné protokoly/providery. Jejich zavedení nesmí vyžadovat přepis UI ani kopírovacího stavového automatu. Detailní pravidla umístění a délky cest určuje následující oddíl 14.

macOS následuje po stabilizaci základních operací na Windows/Linuxu. Zachová se rozdělení UI, enginu a platformních adaptérů.

## 12. Ověřování návrhu při implementaci

- Engine lze spouštět bez Slintu nad testovacím providerem s řízenými chybami a latencí.
- Kritické testy kopie: poškozený cíl, krátké čtení/zápis, prázdný a velký soubor, selhání zápisu/flush/sync/publish, vznik cíle během kopírování a rušení v každé fázi.
- Testy zdroje: append, truncate, změna stejné velikosti a výměna položky během práce. Testy dokumentují limity detekce, nepředstírají snapshot.
- Integrační testy běží zvlášť pro platformní publish pravidla a Windows sharing violations. Názvy s Unicode, ne-UTF-8 na Linuxu, dlouhé cesty a filesystemová citlivost na velikost písmen jsou součástí matice.
- UI ověření zahrnuje souběžnou navigaci a kopírování, saturaci front, ignorování starých generací a práci s velkým adresářem.
- Po zavedení evidence nedokončených úloh testovat restart mezi dokončením zápisu, ověřením a publikováním. Samotné unit testy neprokazují odolnost vůči fyzickému výpadku napájení.

## 13. Zdroje Salamanderu a zbývající implementační volby

Dodané C++ zdroje v `D:\experiments\salamander-main` se průběžně kontrolují jako reference pro každou novou funkci, chování panelů a Windows hraniční případy. Implementace v Rustu vzniká nezávisle; kód, prostředky a řetězce Salamanderu se nekopírují. Konkrétní dohledané reference jsou v doplňujícím návrhu commander ovládání. Unicode a dynamické cesty jsou vlastní závazky produktu, nikoli převzaté limity Salamanderu.

Slint umožňuje desktopovou aplikaci pod více licenčními variantami. Konkrétní licenci projektu a případnou atribuci je potřeba určit před distribucí podle skutečně připnuté verze. [Slint: licence](https://github.com/slint-ui/slint/blob/master/LICENSE.md)

Implementátor může bez změny tohoto návrhu zvolit konkrétní datové struktury modelu, velikosti bloků a dávek, typ async provider rozhraní, počáteční limity workerů a testovací nástroje. Volby ovlivňující uživatelský slib — zejména změnu pravidel přepisování, vypnutí ověřování nebo sledování odkazů — vyžadují vědomou úpravu návrhu.

První implementační plán má podrobně rozpracovat pouze etapu 1. Další etapy zatím určují směr a hranice; jejich plán se zpřesní podle zkušenosti a měření z předchozí etapy. Podrobné průběžné bezpečnostní, integritní a Unicode invarianty jsou v `SECURITY.md`.

## 14. Cesty bez aplikačního limitu a vzdálené zdroje

### Délka a identita

- Aplikace nestanovuje žádnou univerzální maximální délku cesty, názvu ani URL. Nikde nevzniká `MAX_PATH`, `MAX_PATH + 1`, pole o 260 znacích nebo obdobný skrytý limit pro interní identitu, fronty, historii, konfiguraci, clipboard či příkazy operací.
- Ukládání používá dynamické struktury. Velikost se zjišťuje v jednotkách požadovaných konkrétním API: UTF-8 bajty, UTF-16 jednotky nebo providerem definované bajty; počet zobrazených znaků se za velikost bufferu nezaměňuje.
- Fyzicky nekonečná cesta není příslib: konkrétní filesystem, OS či server může mít vlastní omezení celkové cesty, jednotlivé komponenty, protokolu nebo dostupné paměti. Takové omezení pochází od providera a vrací se jako konkrétní chyba. Nikdy se neřeší tichým zkrácením, přepsáním jiné položky nebo převedením do ANSI.
- Zkrácení textu či horizontální posun ve vykresleném řádku je pouze vizuální. Operace, historie a kopírování textu vždy používají celou původní cestu. Ani dočasný cíl či ověřovací manifest nesmí používat zkrácenou identitu.
- Na Windows používat Unicode/long-path kompatibilní API a dle požadavku daného API rozšířené formy cest včetně UNC. Prefixy se nesmějí odstraňovat při skutečných operacích pouze kvůli vzhledu. Budoucí shell integrace musí buď zachovat tuto schopnost, nebo jasně oznámit omezení konkrétní externí operace. [Microsoft: dlouhé cesty](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation)

### Umístění podle providera

Před zavedením prvního vzdáleného zdroje se lokální reprezentace v navigační smlouvě rozšíří na koncept:

`Location = ProviderId + ProviderPath`

`ProviderPath` uchovává bezeztrátovou, providerem vlastněnou identitu: pro lokální filesystem `PathBuf`, pro vzdálený server jeho cestu/token a pravidla kódování. Není to jeden univerzální `String`/URL předávaný lokálnímu `Path::join`. Zobrazený text je samostatná Unicode reprezentace.

Provider v back-endu provádí sestavení a rozlišení cesty, práci s kořenem/rodičem, kódování protokolu, pravidla velikosti písmen a interpretaci odkazů. UI posílá záměr navigace a nepronáší do vzdálených cest lokální oddělovače, písmena disků, `MAX_PATH` či normalizační pravidla Windows. Ověřování existence neprobíhá v UI.

Přihlašovací údaje jsou mimo identitu cesty a zobrazované URL; nepatří do historie, logů ani chybových hlášek. Síťový provider má omezený souběh, časové limity, kooperativní rušení a explicitní politiku opakování. Název souboru je parametr knihovny/protokolu, nikdy shellový příkaz nebo neověřený text vložený do řídicího řádku protokolu. Podpora starších serverových kódování neospravedlňuje tiché poškození Unicode; provider musí kódování vyjednat/zvolit a při nemožnosti bezeztrátového převodu vrátit chybu. [RFC 2640: internacionalizace FTP](https://www.rfc-editor.org/rfc/rfc2640)

### Ověřená kopie mezi providery

Stejný kopírovací engine zpracuje zdroj a cíl typu `Location`. Povinné ověření obsahu platí i při kopii na FTP nebo jiný vzdálený server: je nutný nezávislý úplný čtecí průchod cílem, případně později výslovně definovaný důvěryhodný ekvivalent s doloženou zárukou. Nepřítomnost serverového checksum příkazu není důvod ověřování vypnout; lze znovu číst obsah přes provider.

Schopnosti dočasného cíle, přejmenování/publikování, přepisování a trvalosti se vyhodnocují zvlášť. Vzdálený provider bez potřebné záruky nepředstírá atomickou lokální operaci. Zdroj přesunu se nesmaže před úspěšným ověřením a publikováním cíle. Bezpečný transport a politika přihlašování jsou součást návrhu konkrétního síťového providera.

### Povinné testy cest

- Lokální Unicode cesty pod i nad 260 UTF-16 jednotek na Windows; dlouhé legální komponenty, hluboké stromy a UNC dle dostupnosti prostředí.
- Dlouhé cesty na Linuxu podle schopností filesystemu, bez přeneseného limitu Windows.
- Zachování celé identity při zobrazení, historii, řazení, výběru, chybách a budoucích kopírovacích/ověřovacích operacích.
- Testovací vzdálený provider s cestami výrazně nad `MAX_PATH`, Unicode jmény, vlastními oddělovači a vlastní sémantikou kořene; odmítnutí nechtěné lokální normalizace.
- Chyba skutečného limitu providera nebo paměťového rozpočtu je viditelná a nedestruktivní, bez zkrácení či záměny cíle.

## 15. Platformní rozhraní a oddělené implementace

### Hranice kódu

Platformní vrstva se skládá z malé továrny/fasády `fm-platform`, přenositelného
`fm-platform-common` a samostatných `fm-platform-windows`, `fm-platform-linux`,
později `fm-platform-macos`. Továrna pouze vybere/reexportuje implementaci pro
daný target; neobsahuje implementace filesystemu. Windows Win32 API a Linux
mountinfo/native logika se nikdy nesdílejí v jednom implementačním souboru.

Společný kód obsahuje jen skutečně přenositelné operace přes bezpečné Rust API,
pomocné datové struktury a doménové smlouvy. OS-specifické testy patří také do
samostatných souborů; stejné smluvní testy běží nad zvolenou implementací na každé
podporované platformě. Platformní crate se nekompiluje jako cizí implementace na
jiném targetu. UI ani engine neimportují Win32, `/proc`, konkrétní platformní
crate nebo `cfg` větve s OS logikou.

### Povinné služby první etapy

Ve `fm-core` jsou objektově použitelná rozhraní:

| Rozhraní | Smlouva |
| --- | --- |
| `DirectoryService` | `read_directory(path, cancel, emit)` postupně vrací položky; `entry_metadata(path)` vrací metadata bez otevírání obsahu speciálních souborů |
| `VolumeService` | `list_volumes()` vrací navigační kořeny bez čekání na dotazy na kapacitu jednotlivých svazků |
| `StorageService` | `space_info(path)` vrací kapacitu; `storage_kind(path)` dává konzervativní LocalSsd/Rotational/Network/Unknown hint pro plánování |
| `FileSystem` | Agreguje povinné služby jako `DirectoryService + VolumeService + StorageService`; engine přijímá toto abstraktní rozhraní |
| `IconService` | Backend vrací omezené RGBA ikony podle typu; žádné otevírání zobrazených programů nebo UI I/O |
| `TerminalService` | Spustí výslovně zadaný příkaz v systémovém terminálu se zachyceným pracovním adresářem |
| `PropertyService` | Explicitní systémové vlastnosti nebo poctivý signál pro read-only metadata fallback |
| `LocationService` | Skutečné disky a podporovaná fyzická speciální umístění, písmena disků jen na Windows |
| `OpenService` | Pouze explicitní otevření skutečné položky: adresáře interně, běžné soubory přes systémové asociace; symlinky klasifikuje backend |
| `DesktopServices` | Agregát předchozích pěti desktopových rozhraní; oddělený od `FileSystem` |
| `WatchService` | Ne-rekurzivní systémové změnové hinty a vlastněná subscription; debounce, kontrola a polling patří enginu |
| `SettingsService` | Načtení/verzované atomické uložení typovaných preferencí pouze na backendu |

Každá OS implementace musí dodat tyto služby; může delegovat přenositelné části
na `fm-platform-common`. Podpisy zachovávají bezeztrátové cesty, strukturované
chyby a kooperativní rušení. Synchronní lokální provider API smí engine volat
pouze na pracovních vláknech. Budoucí síťové providery mohou používat nativně
asynchronní I/O; lokální blokující bridge nesmí určovat jejich protokol ani nutit
síťové operace na UI vlákno.

### Služby přidávané spolu s příslušnou funkcí

- `CopyPublicationService`: práce s otevřenou identitou souboru, výhradní dočasné
  vytvoření, synchronizace, publikování bez přepsání a bezpečná výměna. Jednotlivé
  schopnosti jsou explicitní; chybějící záruka vrací `Unsupported` před mutací.
- `TrashService`: přesun do koše s výsledkem a dostupnými možnostmi obnovy.
- `MountService`: připojení a bezpečné odpojení zařízení včetně potřebné uživatelské interakce.
- Další schopnosti asociací a explicitního otevření rozšíří existující desktopové služby; žádné sestavování shellových příkazů z názvů souborů.

Tyto budoucí služby se dnes nenahrazují nefunkčními stuby ani falešným úspěchem.
Přibudou s implementací, deklarací schopností a smluvními testy dané funkce.
MacOS nebo další OS získá funkce implementací stejných smluv a průchodem
společnými i vlastními OS testy. FTP/FTPS/SFTP jsou nezávislí filesystemoví
provideři nad těmito hranicemi, nikoli varianty Windows implementace.

## 16. Styl zdrojového kódu

Závazný stručný dokument je `CODE_STYLE.md`: oficiální veřejný Rust Style Guide,
Rust API Guidelines a výstup `rustfmt`, cíl 100 znaků na řádek, čtyřmezerové
odsazení, idiomatické názvy a minimální veřejný povrch s dokumentovanými chybami.
Ručně psané zdrojové soubory mají nejvýše 100 000 bajtů; generovaný kód,
lockfile, třetí strany a média jsou výjimky. Počet řádků je vodítko pro rozdělení
odpovědností, nikoli další pevný limit. Velikost kontroluje
platformně oddělené `scripts/check-source-size.ps1` / `scripts/check-source-size.sh`
lokálně i v CI. Platformní oddělení, bezpečné API,
Unicode a asynchronní hranice jsou součást stylu a code review, nikoli jen formátování.

## 17. Multiselekce, dávky a společná fronta operací

### Výběr a neměnný požadavek

Každý panel má nezávislý kurzor, kotvu rozsahu a multiselekci. Návrh zahrnuje
Space/Insert, Ctrl+klik, Shift+rozsah a výběr všeho. Výběr je navázán na skutečné
identity položek a generaci výpisu, nikoli na indexy řádků nebo zkrácené názvy.
Velké rozsahy a materializace seznamu položek se zpracují v back-endu; UI nedělá
synchronní průchod statisíci soubory.

Při zadání operace vzniká neměnný snapshot výběru a cílového umístění. Back-end
z něj vytvoří plán; následná navigace, změna aktivního panelu či výběru nesmí
přesměrovat už zadanou úlohu. Před skutečným otevřením se revalidují dostupné
identity a podmínky operace. Snapshot výběru není snapshot obsahu měnícího se
souboru; pravidla stability zdroje a povinného ověření zůstávají platná.

### Jednotný model úloh

- `OperationRequest` zahrnuje `Copy`, `Move` a `Delete`; další typy se přidávají
  samostatnými handlery ve stejném back-endovém scheduleru.
- `EnqueueOperation(request_id, selection_snapshot, destination, policy)` má
  neblokující vstup a potvrzení `OperationAccepted { request_id, job_id }`, nebo
  konkrétní odmítnutí. UI nepředstírá zařazení před přijetím. Opakovaný request_id
  nezaloží omylem druhou destruktivní úlohu.
- V první verzi se deduplikační identita uchovává po dobu procesu i po dokončení
  úlohy. Obnova po restartu nesmí automaticky přehrát mutace; případná budoucí
  perzistentní fronta uloží request_id a přijetí transakčně společně.
- Fronta i autoritativní registr úloh patří enginu. Úloha přežije změnu/zavření
  panelu průběhu; není vlastněná dočasným dialogem nebo souborovým panelem.
- Během aktivní operace zůstává možné prohlížet oba panely, měnit výběr a zadat
  další Copy/Move/Delete. Kapacitně omezený vstup nikdy nezablokuje callback UI;
  při zaplnění vrátí jasné odmítnutí místo tichého zahození požadavku.
- Úlohy mají vlastní identitu, chyby, rozhodnutí o konfliktech a zrušení. Zrušení
  jedné úlohy neruší další úlohy ani navigaci. Na rozhodnutí uživatele čeká stavový
  automat, nikoli zablokované vlákno.

Základní stavy: `Queued`, `Planning`, `Running`, `WaitingForDecision`,
`WaitingForDependency`, `CancelRequested`, `Cancelled`, `Completed`,
`CompletedWithErrors`, `Failed`. Běžící kopie má navíc fáze `Copying`, `Syncing`,
`Verifying`, `Publishing`. Výsledek celé dávky a výsledky jednotlivých souborů
jsou oddělené; přeskočené či chybné soubory se nezamění za úplný úspěch.

### Průběh

Seznam operací ukazuje čekající i aktivní úlohy a u každé počet souborů,
aktuální soubor, zpracované bajty, fázi, dostupný odhad a chyby. Kopie zobrazuje
samostatně přenos a ověřování. Soubor se počítá jako dokončený až po úspěšném
ověření a publikování; poslední zapsaný bajt neznamená 100 % hotové operace.

Pro celé stromy se plán a součty získávají postupně, bez blokujícího úplného
předběžného skenu. Neznámý celkový počet/velikost používá neurčitý průběh;
nevymýšlí se falešné procento. Časté průběžné hodnoty se slučují na poslední stav
a UI se aktualizuje dávkovaně, ale přijetí, chyby a terminální výsledky jsou
doručované spolehlivě.

### Pořadí a bezpečný souběh

Nové zadání a provádění jsou nezávislé: přijetí nové úlohy nečeká na konec staré.
Počáteční scheduler používá konzervativní FIFO s nejvýše jednou aktivní mutující
operací; uvnitř jedné kopírovací operace smí běžet několik nezávislých souborů
podle rozpočtů v oddílu 8. Čekání operace na rozhodnutí zabírá logický slot, nikoli pracovní
vlákno; nové požadavky se nadále přijímají. Navigace má vlastní pracovní kapacitu.
Budoucí paralelní zpracování povolí jen nezávislé operace podle zdrojového/cílového
providera, zařízení a konfliktů, se stejnými sdílenými I/O limity; samotné spuštění
tisíců Tokio tasks není plánování. Nové kopírování/přesun/mazání lze zařadit i během
kopírování nebo jeho ověřování. Nezávislé soubory běžící operace mohou pokračovat,
pokud jejich výsledek nezávisí na čekajícím rozhodnutí; cílové identity a zámky se
při čekání znovu ověří podle oddílu 25.

Kolidující cesty, podstromy a zjistitelné aliasy mají chráněné pořadí. Zejména
nesmí souběžné mazání/přesun zdroje zneplatnit běžící kopii ani dvě úlohy
nekontrolovaně publikovat do stejného cíle. Při selhání kopie se navazující
destruktivní úloha nad stejnými daty automaticky nespustí; přejde do chyby nebo
čekání na nové vědomé rozhodnutí. Pravidla identity/kolizí určuje provider,
nikoli globální převod Unicode názvů na malá písmena.

Závislost chránící destruktivní krok je splněná pouze úplným `Completed`
předchozí operace, včetně ověření/publikování všech požadovaných souborů.
`CompletedWithErrors`, `Failed` nebo `Cancelled` ji nesplňují.

Přesun mezi filesystémy je ověřená kopie a teprve poté odstranění zdroje.
Přesun v rámci jednoho filesystemu může použít jeho bezpečný rename kontrakt.
Mazání má vlastní deklarovanou politiku koše/trvalého odstranění. Rušení nevrací
již dokončené destruktivní kroky zpět a UI takový rollback neslibuje.

### Povinné přejímací testy

- Multiselekce a dávka používají přesně zachycené položky i po přepnutí panelu,
  navigaci, změně řazení nebo změně aktuálního výběru.
- Zablokovaná kopie nebrání přijetí další kopie/přesunu/mazání ani odezvě UI;
  všechny potvrzené požadavky mají dohledatelný JobId a správné pořadí.
- Průběh zahrnuje ověřování; poškození jediného cíle nesmí dát celé dávce stav
  úspěchu. Duplicity request_id, zaplnění front a zahlcení progress událostmi
  nezpůsobí zdvojenou operaci nebo ztrátu koncového výsledku.
- Kolidující operace a závislosti jsou serializované; selhání předchozí kopie
  nezpůsobí automatické smazání jediného platného zdroje.
- Zrušení jedné dávky je izolované; čekání na konflikt neblokuje UI/vlákno;
  krátké zápisy, plný disk, změna zdroje a výpadek serveru mají konkrétní výsledky.

## 18. Sjednocený rozsah a aktuální ovládání

Tato tabulka rozlišuje hotový základ od právě doplňovaných částí a budoucích etap.
Provedené testy nejsou náhradou za dosud neprovedená měření či nasazení na Mint.

| Oblast | Stav / závazek |
| --- | --- |
| Rust/Slint/Tokio, jeden proces a oddělené crates | Implementováno; UI nezná konkrétní engine ani OS API |
| Windows x64, Linux | Implementováno a testováno na Windows a WSL Ubuntu 24.04; samostatná kvalifikace Mint zbývá |
| Unicode, raw identity, cesty bez aplikačního limitu | Závazné; testy včetně dlouhých Windows cest a ne-UTF-8 Linux názvů |
| Dva panely, svazky, historie, postupná metadata, multiselekce | Implementováno; řádky jsou virtualizované |
| F1–F10, editor příkazů, vlastnosti, ikony, umístění, `..` | Implementováno; rozsah funkčních tlačítek níže |
| Výpočet velikostí a přepínání směru řazení | Implementováno včetně sdíleného plánování SSD/HDD/síť/Unknown z oddílu 19 |
| Automatická obnova a zachování rozpracovaného UI | Implementováno; nativní události i fallback polling |
| Odstranění horního pruhu, témata, hustota, ukládání preferencí | Implementováno podle oddílu 21; shell picker je na všech aktuálně podporovaných platformách |
| Kopírování/přesun/mazání a jejich společná fronta | Následující etapy podle uživatelem zvoleného read-only rozsahu; ověření nelze obejít |
| FTP/FTPS/SFTP, archivy, macOS | Připravené hranice providerů/platformních služeb; implementace později |
| Vestavěný viewer a obecné VFS | Finalizovaný návrh dynamických pluginů: text/source, nativní OS provider, ZIP/GZIP/TAR.GZ/7z; implementace následuje podle oddílů 22–23 |

| Vstup | Chování |
| --- | --- |
| Tab | Přepne aktivní souborový panel |
| Levé kliknutí / Ctrl+klik | Kurzor a aktivní panel / přepnutí výběru položky; samotný výběr nic nespouští |
| Enter / levý dvojklik bez modifikátorů | Vstup do adresáře nebo otevření běžného souboru výchozí systémovou aplikací; odkazy na adresáře zůstávají v panelu |
| Backspace / dvojklik na `..` | Navigace k rodiči; `..` není skutečný soubor ani součást výběru |
| Space / Insert | Multivýběr podle identity; Insert také posune kurzor |
| Alt+Left / Alt+Right, Ctrl+R | Historie / nový výpis aktivního umístění |
| Kliknutí na záhlaví Name/Size/Modified/Type | První volba sloupce vzestupně; opakované kliknutí přepíná asc/desc; šipka ukazuje směr |
| Ctrl+Shift+S | Výpočet všech složek aktivního panelu bez ohledu na výběr; opakování ruší |
| Alt+Enter, také Ctrl+Alt+Enter | Vlastnosti vybraných položek, jinak skutečné položky pod kurzorem; samotné `..` nic nespouští |
| Ctrl+[ / Ctrl+] | Vloží celou cestu levého/pravého panelu jako samostatný bezpečně citovaný argument |
| Enter / Run v příkazovém editoru | Explicitně zadaný příkaz spustí v externím systémovém terminálu, cwd se zachytí při zadání |
| Výběr vlevo od příkazového editoru | Windows PowerShell/cmd; POSIX Terminal/Terminal (sudo). Běžná Windows volba se ukládá, zvýšený POSIX režim pouze do konce relace |
| Shift+písmeno disku | Pouze Windows a zaměřený seznam souborů; v editorech zůstává běžné psaní |
| Lišta disků a míst nad každým panelem | Cílí pevně na svůj panel; disk obnoví jeho zapamatovaný adresář, speciální místo otevře přesnou cestu |
| Options / menu | Volba tématu a hustoty; About je v menu, nikoli v samostatném horním řádku |

Spodní lišta zachovává referenční pořadí F1 Help, F2 Rename, F3 View, F4 Edit,
F5 Copy, F6 Move, F7 Create directory, F8 Delete, F9 User menu, F10 Menu.
Dosud neimplementované akce jsou skutečně zakázané; klávesy neobcházejí stav
tlačítek. F10 otevírá menu, ukončení je jeho samostatná položka. Příkazový editor
je výslovná akce uživatele a může přes systémový shell měnit soubory; read-only
rozsah se týká interních souborových operací manažeru.

Windows používá důvěryhodné absolutní systémové cesty pro PowerShell i cmd.
PowerShell předává Unicode přes `-EncodedCommand`; cmd přes přesný Unicode
command line s vypnutým AutoRun a delayed expansion. Linux spouští podporované
systémové terminály se samostatnými argumenty shellu; sudo pouze při výslovně
zvoleném Elevated režimu. Názvy vložené aplikací se nesestavují do kódu. Nejednoznačné
místo uvnitř uvozovek, interpolace nebo jiné nepodporované shellové konstrukce
se při vkládání odmítne bez změny rozepsaného příkazu. Vložení nikdy samo nic
nespustí; záměrný uživatelský `eval` může argumenty znovu interpretovat.

Cmd nepodporuje libovolný bezpečný procentní literál stejným způsobem jako
PowerShell. Cesta obsahující `%`, uvozovku, newline nebo vyžadující citování
a končící zpětným lomítkem se pro vložení do cmd výslovně odmítne s možností
zvolit PowerShell; její identita se nezkracuje ani
tiše nepřepisuje. Cmd také nesmí potichu změnit UNC pracovní adresář na lokální:
nepodporované cwd vrátí chybu s doporučením PowerShellu.

Sudo komunikuje s heslem přímo v externím terminálu, nikdy přes UI nebo `-S`.
Standardní režim je výchozí při novém spuštění; Elevated se neukládá. Běžná volba
PowerShell/cmd je součást UserPreferences. Připravené POSIX UI není tvrzení, že
už existuje macOS launcher; ten musí dodat samostatný budoucí adaptér.

Systémové ikony vznikají na backendu podle typu/přípony, ne extrakcí ze spuštěného
či zobrazeného programu. Přenos jsou omezené RGBA pixely a UI kešuje Slint Image.
Chybějící ikona má generovaný fallback. Windows používá typové Shell/stock ikony,
Linux zatím omezené instalované PNG motivy. Vícenásobné či nepodporované nativní
vlastnosti mají read-only metadata fallback, nikdy implicitní otevření souboru.

Scroll zachovává fyziku Slint ListView/Flickable: dojezd prstu/touchpadu a krátkou
animaci kolečka přibližně 180 ms. Nepřidává se druhá animace content-y. Krátký
dojezd kolečka je přijatelný, pokud nepřekáží přesnému ovládání; aktualizace ikon,
metadat a automatický refresh nesmějí vracet viewport ke kurzoru.

## 19. Velikosti složek, férovost a typ úložiště

Součet používá logické délky běžných souborů, nikoli alokované bloky. Explicitní
symlinky/junctions a speciální soubory se nepřecházejí. Průběžná, zrušená nebo
chybová hodnota je zřetelně neúplná, ne úspěšná nula. Sčítání kontroluje přetečení.
Průchod je živý odhad měnícího se stromu; path-based read-only API neslibuje
atomický snapshot nebo bezpečnostní záruku pro budoucí destruktivní průchody.

Hotová část používá rušení po položkách/adresářích, průběh nejvýše přibližně
10krát za sekundu na kořen a omezenou frontu cest (16 MiB na průchod). Vyčerpání
rozpočtu dává explicitně neúplný výsledek. Jednotlivá cesta nemá pevný limit.
Řazení je samostatný backendový `Resort` nad přijatým snapshotem: zachová ID
položek a hotové bezchybné velikosti. Neúplné hodnoty se nepoužijí jako přesná
velikost. Refresh načte skutečná aktuální data a staré výpočty zneplatní.

Nově požadovaná politika plánování:

| Úložiště | Aktivní I/O pro výpočet | Důvod |
| --- | --- | --- |
| Prokazatelně lokální SSD / NVMe | Malá omezená dávka, výchozí strop 4 | Menší složka může dokončit souběžně s velkou; nezavádět úlohu pro každou složku/soubor |
| Rotační HDD | 1 | Omezit souběžné seeky; férově střídat omezené úseky aktivních průchodů |
| Síťový disk, UNC, NFS/SMB, vzdálený provider | 1 | Paralelní požadavky mohou zahltit server nebo zvýšit latenci |
| Neznámé, virtuální či neprůkazné médium | 1 | Při nejistotě nepředpokládat SSD podle naměřené rychlosti nebo cache |

Detekce patří platformním službám a běží pouze na backendu. Windows ověřuje
síťový charakter a vlastnosti storage zařízení; Linux mount/device informace a
rotaci lokálního blokového zařízení. Nedostupná či nejednoznačná informace vede
k `Unknown`. Cesta nad lokálním SSD může vést přes další mount na jiné médium;
nejistá či smíšená topologie nesmí automaticky získat agresivní SSD rozpočet.

Rozpočet je sdílený mezi panely, ne samostatné čtyři workery na každý panel.
Zejména dva panely nad stejným síťovým serverem nesmějí násobit jeho zátěž.
Průchod má omezený počet rozpracovaných složek a férové kratší pracovní úseky;
nepoužije neomezeně vláken, otevřených adresářů ani úloh v Tokio poolu. Jedna
neukončitelná systémová operace může držet svůj slot; okamžité zastavení OS I/O
se neslibuje. Navigace, watchery a render mají oddělenou pracovní kapacitu.

Přejímací testy: zablokovaná velká složka na SSD nebrání malé v aktivní dávce,
strop souběhu se nepřekročí, HDD/Unknown/síť nikdy neprovádí více I/O současně,
dva panely sdílejí síťový rozpočet, detekční chyba volí konzervativní režim a
zrušení propustí čekající práci bez falešných terminálních výsledků.

## 20. Watchery, dávkovaná obnova a skutečně viditelný snapshot

Každá platforma implementuje `WatchService` v samostatném souboru. Aktuální
Windows adaptér používá nativní directory notifications a Linux inotify přes
notify 8.2.1; macOS musí dodat svůj adaptér. Sledují se okamžité obsahy viditelných
adresářů. Create/delete/rename/změna dat či metadat/error/overflow jsou hinty
k opětovnému výpisu. Read/access události se ignorují, aby samotný výpis
nevyvolával nekonečnou smyčku.

Engine uchovává jen poslední cíl a dirty stav každého panelu. Obnova proběhne po
1 sekundě klidu, nejpozději po 3 sekundách nepřetržitých změn. Dva omezené
backendové workery vlastní registrace, rušení i polling. Každých 15 sekund
porovnají skutečná jména, typy, délky souborů a časy změn. Nezměněná kontrola
refresh nevyvolá. Obnovení dříve nedostupných metadat jej vyvolá právě tak,
aby se nové hodnoty dostaly do UI. Vypočtený součet složky není fyzická délka
adresáře a do porovnání se nezaměňuje.

Polling zůstává aktivní i po úspěšné nativní registraci: síťové filesystémy a
některé Windows overflow/read-error případy nemusí notifikaci doručit. Hints
neslouží jako důkaz integrity. Odpojené médium/OS chyby mohou obnovu oddálit.

UI potvrzuje skutečně přijatý výpis příkazem `PaneCommitted(VisibleDirectory)`
se sdíleným `Arc` snapshotem. Výpis, který worker dokončil, ale UI jej jako starý
ignorovalo, se nesmí stát autoritativním viditelným stavem. Omezenou vstupní
frontu jistí krátké opakování potvrzení po 100 ms. Hint nese panel/generaci;
zastaralý hint ani probíhající uživatelskou navigaci nepřepisuje.

Obnova stejného umístění připravuje nová data za stále použitelnými starými
řádky a výsledek vymění najednou. Zachová surové identity výběru/kurzoru,
viewport, aktivní panel a rozepsaný příkaz včetně caret/fokusu. Při chybě zůstává
poslední použitelný snapshot; polling později obnovu zkusí znovu. Sledování se
ukončuje na backendu a plná výstupní fronta nesmí bránit shutdownu.

## 21. Témata, hustota ovládání a uživatelské nastavení

Horní branding řádek FM/About se odstraní. Tlačítko **Options** bude u příkazového
řádku a dostupné také v menu. Okno nastavení obsahuje nezávislé výběry:

| Volba | Varianty |
| --- | --- |
| Téma | Blue (dosavadní modré, výchozí), Dark (neutrální tmavé), Light (světlé) |
| Hustota | Desktop / mouse (kompaktní), Touch (větší rozestupy a dotykové cíle) |
| Font | Nainstalovaná rodina nebo výchozí font aplikace |
| Základní velikost | Nezávislá hodnota Desktop a Touch,8–32 logických px |

Společná Slint Theme vrstva poskytne sémantické barvy pro pozadí, panely, text,
neaktivní/aktivní stav, výběr, fokus, chyby a zakázané ovládání. Light přepne také
schéma standardních Slint widgetů; nestačí změnit jen pozadí seznamu. Všechna
témata zachovají čitelný výběr, aktivní panel a indikaci chyby. Statické toolbar
ikony jsou vložené prostředky, aby vykreslování neotvíralo soubory z UI.

Desktop používá22px řádky a úsporné rozestupy. Touch používá minimálně
40 logických px pro souborové řádky a hlavní ovládací cíle, větší výšky editorů,
tlačítek a padding. Jedna sada metrik řídí render i výpočty scrollu/reveal/hit
testů; kód nesmí předpokládat pevnou výšku po přepnutí hustoty. Změna tématu ani
hustoty nesmí měnit aktuální cesty, výběr, operace nebo rozepsaný příkaz.

Okamžitý náhled je čistá UI operace. Perzistence je realizační volba backendu:
`SettingsService` ukládá typované `UserPreferences` ve verzovaném souboru
(v3; migrace v1/v2). Windows použije uživatelské LocalAppData, Linux absolutní XDG_CONFIG_HOME
nebo ~/.config; platformní rozhodnutí zůstávají v oddělených OS souborech.
Konfigurace neobsahuje spustitelné příkazy nebo přístupové údaje.
Obsahuje také běžnou volbu Windows shellu; zvýšený POSIX režim se neukládá.

Načítání/zápis/úklid běží na workeru, ne v UI. Chybějící soubor znamená výchozí
hodnoty; chybná či nepodporovaná data vrací chybu bez přepsání původního souboru.
Čtení je omezené velikostí, ověřuje otevřený běžný soubor a nesmí uváznout na
FIFO, zařízení či odkazu. Uložení používá vlastní výhradně vytvořený dočasný
soubor, dokončení zápisu/synchronizaci a atomické publikování; nikdy nejprve
nevymaže existující nastavení. Linux používá soukromá oprávnění.

Požadavky mají monotónní `PreferencesRevision`. Opožděné načtení nesmí přepsat
novější uživatelskou volbu; starší uložení nesmí zvítězit nad novějším. Engine
serializuje zápisy a drží nejvýše jeden běžící a poslední čekající stav. Selhání
uložení nezruší aktuální náhled a zobrazí možnost opakování. Po zavření okna
proběhne omezené dokončení posledního přijatého uložení (cílově do 2 sekund);
blokující OS volání ani pád napájení nedostávají falešnou záruku durability.

Testy zahrnou všech šest kombinací tématu/hustoty, skutečný render světlých i
tmavých ploch, touch hit test a scroll, zachování fokusu/caret, závod load/save,
rychlou sérii změn, chybu a opakování uložení, změnu a okamžité zavření, poškozený
soubor, odkazy/FIFO/zařízení a Unicode i dlouhou cestu ke konfiguraci.

Příkazový draft je při prvním psaní nebo vložení cesty navázaný na konkrétní
shellovou gramatiku. Opožděně načtené nastavení nesmí gramatiku neprázdného draftu
změnit. Přepnutí PowerShell/cmd s neprázdným příkazem se odmítne s pokynem nejprve
vyčistit řádek; již citovaná cesta s `&` se nesmí změnit ve spustitelnou syntaxi.
Přepnutí Standard/sudo v rámci POSIX gramatiky je výslovná změna oprávnění,
stále vyžaduje Enter/Run a nezapisuje zvýšený režim do konfigurace.

## 22. Vestavěný viewer: dynamické pluginy pro text/source a později Markdown

Aktuální [finální plugin kontrakt](2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
stanoví samostatné knihovny v `viewers/`, discovery při startu, verzované C ABI,
backendové leases a společný Slint renderer. První plugin poskytne plain text
a zvýraznění syntaxe známých zdrojových jazyků. Odebraný plugin odebere příslušnou funkci;
F3 se aktivuje až se skutečnou implementací. UI neobsahuje skrytý parser fallback.

Čtení, dekódování, indexování, syntax, hledání, velké výběry a cleanup patří
backendu. UI vykreslí omezené UTF-8 stránky a sémantické spany se source map,
generation a revision. Unicode shaping, výběr a bidi zachovají společný kontext
řádku; jednotlivé tokeny se nesmějí rozpadnout do nezávislých Text widgetů.
Sekvenční stream nepředstírá levný random access, velký soubor se nenačítá celý.

Markdown přijde jako další plugin s vyjednaným block modelem. Zachová zákaz
aktivního HTML/scriptu a automatického stahování i hostitelskou LinkPolicy.
Podrobnosti obsahových kontraktů, Unicode, cache, source změn a reference
Salamanderu zůstávají v [původním viewer/VFS návrhu](2026-11-04-viewer-vfs-design.md).

## 23. VFS a migrace na platformní nativní pluginy

Každý filesystem včetně nativního je samostatný dynamický plugin ve
`filesystems/`. Windows, Linux a později macOS mají oddělené implementace a testy.
První archivní balíčky jsou ZIP, GZIP, TAR (také TAR.GZ/TGZ) a 7z; procházení
členů a F3 jsou read-only. GZIP zpřístupní jeden logický stream, TAR.GZ strom;
TAR má vlastní decoder helper a nezávisí na přítomnosti GZIP pluginu.

Současné PathBuf rozhraní se migruje přes adaptér na provider registry,
MountId/NodeRef a content broker. Provider vlastní namespace, parent/root,
case pravidla, persistent address i raw identity. Display text není OS cesta;
není žádný aplikační MAX_PATH. Capabilities pravdivě rozlišují čtení/seek/watch,
metadata a budoucí mutace. Odebraný provider ponechá karty/záložky nedostupné,
neztratí jejich uložené adresy. Chybějící native plugin odebere lokální browsing;
vestavěné host služby nastavení, terminálu, ikon a loaderu zůstanou funkční.

Archivy používají backing leases, bounded index/dekompresi, bezpečné opaque
identity členů a watch invalidaci. Neextrahují automaticky cesty ani odkazy.
Budoucí extraction/copy vyžaduje společný BLAKE3 destination readback a bezpečné
publish; CRC archivu není náhrada. Registry zůstává budoucí Windows plugin
s klíči a typovanými hodnotami, vzdálené zdroje dalšími capability implementacemi.

Plný kontrakt, balení, životnost, bezpečnost a přejímací matice jsou v
[novém návrhu dynamických pluginů](2026-11-05-dynamic-viewer-filesystem-plugins-design.md).
Nativní DLL/SO jsou důvěryhodný kód s právy aplikace, nikoli sandbox. Knihovny
zůstanou načtené do ukončení procesu, instalace/odebrání se uplatní po restartu.
Každý plugin má samostatný repozitář, build a vlastní testy/CI. Sdílí pouze veřejné
API/SDK, ne interní app crates. Vývoj, build, test ani instalace pluginu nesmějí
upravovat zdrojáky hlavní aplikace; rozšíření API je samostatná změna hostitele.

## 24. Opakované sudo a životní cyklus terminálové relace

Podrobný návrh je v [návrhu terminálových relací](2026-11-04-terminal-sessions-design.md).
Automatické znovupoužití není součást hotového launcheru: aktuální spuštění
vytváří nové externí okno/TTY. Ponechaný interaktivní root shell lze používat
ručně, ale editor manažeru do něj další příkazy automaticky neposílá. Návrh
tento rozdíl výslovně nezaměňuje za fungující cache.

Cílové řešení drží jeden **neprivilegovaný** broker ve stejném externím terminálu.
Každý výslovný příkaz projde skutečným systémovým sudo v téže relaci, čímž lze
využít systémovou cache podle místní politiky. Aplikace neukládá heslo, nepoužívá
`sudo -S`, nemění sudoers/NOPASSWD a neobnovuje oprávnění neomezeným periodickým
`sudo -v`. Systémový timeout a nové výzvy k ověření zůstávají respektované.

Příkazy a status používají vlastní omezený IPC kanál oddělený od nedůvěryhodného
výstupu terminálu. Broker i aplikace se ověří kernelovou identitou procesu/UID
a životností původního procesu; samotný user-owned socket či UUID ve veřejném
argv nestačí. Bez prokazatelného bezpečného bootstrapu konkrétního terminálu
adapter nesmí slibovat reuse. Heslo jde přímo mezi uživatelem, terminálem a sudo.

Další Enter smí poslat příkaz jen do idle relace; nesmí jej vstříknout do právě
běžícího editoru, programu nebo password promptu. Obsazená relace vrátí poctivý
Busy stav, nevytváří neomezenou frontu root příkazů. Operace se nezkouší vykonat
nejprve neinteraktivně a při neúspěchu podruhé: nenulový exit může být výsledkem
už vykonaného příkazu, nikoli chybějící autentizace.

Relace má explicitní zavření, omezenou nečinnost a vazbu na životnost aplikace.
Reused session není nový Child a nesmí spotřebovávat nové sloty/reaper záznamy.
Při odpojení během příkazu se rozlišuje neznámý výsledek od potvrzeného dokončení;
retry nesmí zdvojit destruktivní příkaz. Systémová globální sudo cache se
automaticky nemaže pomocí `-K`; dopady revokace závisejí na místní sudoers politice.

## 25. Konflikty při kopírování/přesunu a potvrzení přepsání

Jde o povinnou součást budoucích zápisových operací, nikoli povolení nyní aktivovat
neověřenou F5/F6 implementaci. Výchozí politika je nepřepisovat. Existující cíl
vyvolá strukturovaný konflikt a úloha přejde do `WaitingForDecision`; UI ani
backendový worker nečeká synchronně na dialog a další operace lze zařazovat.

Dialog porovná zdroj a cíl vedle sebe:

- název a úplnou surovou cestu / provider-owned reprezentaci bez změny identity;
- provider, typ položky (soubor/složka/odkaz/jiný objekt) a případnou kolizi typů;
- přesnou známou velikost v bajtech i čitelnou jednotku, neznámou hodnotu výslovně;
- datum a čas změny včetně časové zóny/rozlišení podle dostupných metadat;
- podle schopností další užitečná metadata (např. read-only stav), bez předstírání
  znalosti úplné rekurzivní velikosti složky nebo hash shody jen ze stejné délky.

Akce: **Přepsat / Přeskočit / Jiné jméno / Zrušit**. Případné **pro všechny tyto
konflikty** je výslovná politika aktuální dávky s jasným rozsahem, ne trvalý
globální souhlas pro nové úlohy. Kolize soubor/složka/odkaz a sloučení adresářů
mají vlastní schopnosti a rozhodnutí; prosté přepsání souboru je nesmí potichu
proměnit v rekurzivní mazání nebo následování odkazu.

Konflikt nese JobId, verzi rozhodnutí a zachycené identity/metadatový stav.
Odpověď na starý nebo už vyřešený dialog se odmítne. Před publikováním se cíl
znovu ověří; změněná identita nebo stav vyvolá nový konflikt, ne použití starého
souhlasu na jiný soubor. Hodnoty v dialogu se získávají na backendu a nesmějí být
jen neověřenou kopií zastaralého panelu.

Při přepsání se nejprve dokončí vlastní dočasný cíl, synchronizace a nezávislý
kontrolní readback s délkou a kryptografickým hashem. Existující cíl se předem
nemaže ani netruncuje. Teprve potom se použije deklarované/testované atomické
publikování či bezpečná výměna daného provideru. Chybějící záruka znamená
Unsupported, ne destruktivní fallback. U přesunu mezi filesystémy zůstává zdroj,
dokud cílová kopie není ověřená a úspěšně publikovaná; rename v rámci filesystemu
také musí respektovat kolize a platný souhlas.

Testy zahrnou všechny volby a jejich rozsah, metadata/neznámé velikosti/Unicode,
rozdílné typy a odkazy, vznik či výměnu cíle během dialogu i během kopie,
zastaralou odpověď, chybu/rušení po ověření před publikací a zachování původního
cíle při selhání. Datum ani stejná velikost nenahrazují ověření obsahu.

Atomická výměna sama o sobě není podmíněná výměna potvrzené identity. Provider,
který slibuje ochranu proti nahrazení cíle po poslední kontrole, musí mít zvláštní
schopnost `ConditionalOverwrite(expected_identity_or_version, staged_target)`
s definovaným linearizačním bodem, případně skutečně výhradní namespace kontrakt.
Pouhé stat následované rename/ReplaceFile tento slib nesplňuje. Bez této
schopnosti je silné podmíněné přepsání Unsupported; případná slabší politika
„nahradit aktuální položku na této cestě“ musí být zvlášť pojmenovaná a výslovně
zvolená, nikdy tichý fallback. Test musí změnit cíl i po poslední revalidaci těsně
před publikováním a prokázat, že nový soubor nepřepsal starý souhlas.

## 26. Kompaktní ovládání a tooltipy

Trvalá vysvětlující nápověda nemá zabírat místo souborům a příkazům. Spodní
pruh s připomínkami zkratek, nápověda v prázdném příkazovém editoru a text ACTIVE
se odstraní; zvýraznění aktivního panelu, chyby a dostupná
nápověda F1 zůstávají. Navigační nástroje, hlavičky řazení, systémová umístění,
nastavení a příkazové ovládání mají skutečné tooltipy i přístupné popisky.

Volba shellu je malé ikonové tlačítko úplně vlevo, bezprostředně před příkazovým
editorem. Tooltip vysvětlí aktuální PowerShell/cmd nebo Terminal/(sudo), pracovní
adresář a použití. Rozbalená nabídka zachová celé čitelné názvy, ovládání
klávesnicí a rozlišení režimu sudo. Přepnutí samo nic nespouští; ochrana gramatiky
rozepsaného příkazu nadále platí. Aktuální cwd zůstane vedle editoru jako celý
read-only prompt bez výpustky a pevného omezení šířky. Výjimečně dlouhá cesta
má vodorovný posuvný výřez; příkazové ovládání zůstává v jediném řádku.
Prompt není součástí spouštěného příkazu.
Dotyková varianta používá po požadavku na zmenšení40px základní cíle.

Profil Desktop/mouse je výrazně hustší:22px řádky,24px ovladače,14px ikony,
6px okraje a4px mezery. Dotykový profil má40px řádky/ovladače,48px hlavní lišty,
10px okraje,6px mezery a20px ikony. Ikony souborů a složek se centrují podle výšky řádku. Desktopová
tlačítka disků mají46px s krátkým `C:`, speciální místa24px ikonové cíle s celým
popiskem v tooltipu; při dotyku zůstávají celé názvy. Rozbalovací šipka je SVG,
aby nezávisela na dostupnosti symbolu v systémovém fontu.

Options nastavuje `close-policy: close-on-click-outside`, protože výchozí
`close-on-click` Slintu zavírá popup i po kliknutí uvnitř. Test skutečných vstupů
musí projít změnou Dark→Touch→Light, dalším ovládáním, Escape, novým otevřením
a Done; samotné zavolání callbacku nebo screenshot toto chování neověřuje.

Tlačítka disků zůstávají krátká. Tooltip obsahuje na Windows skutečný název
svazku a volné místo; Linux a budoucí macOS nepotřebují umělé písmeno nebo název
svazku, zobrazí kapacitu. Rozbalovací nabídka zobrazuje vedle písmene/kořene také
název, pokud jej platforma poskytuje, a volné místo. Šířka rozbalené nabídky je
nezávislá na zavřeném ovladači. Neznámé, načítané nebo nedostupné údaje mají
výslovný stav; chyba se nesmí změnit na zdánlivých 0 B volného místa.

`VolumeService::volume_label(path)` má výchozí `Ok(None)`; Windows poskytne
vlastní implementaci v samostatném souboru s dynamickými UTF-16 buffery, bez
MAX_PATH. `VolumeDetails` odděluje výsledek názvu a kapacity, aby dílčí chyba
nezrušila druhý údaj. `VolumeDetailsUpdated` nese ID i původní nativní kořen.
Oddělený backendový řadič ponechá rychlý výpis kořenů nezávislý na dotazech na
jednotlivé disky; metadata mají dva skutečně obsazené pracovní sloty, deduplikaci,
cache a pravidelnou obnovu přibližně po 45 s. Pomalé síťové a odpojené svazky
nesmějí držet kapacitu navigace; probíhající blokující OS volání si ponechá svůj
permit do skutečného návratu. Zastaralé výsledky odstraněných kořenů se zahodí.

Kapacita Windows používá vlastní `GetDiskFreeSpaceExW` s dynamickým zjištěním
kořene a volným místem dostupným aktuálnímu uživateli včetně kvót; původní
fixed-buffer cesta knihovny fs2 se pro Windows nepoužívá. Chybějící cesta zůstává
chybou. Dotazy na název i kapacitu potlačí dialogy nedostupných médií pouze na
svém pracovním vlákně a předchozí režim vždy obnoví. Expirace cache má vlastní
deadline, aby periodický tick nezměnil 45s interval na 90s.

Otevřená nabídka disku se zruší při změně přesné posloupnosti ID a nativních
kořenů, i když se dvě různé nevalidní Unicode cesty zobrazují totožně. Aliasy
jako Home uvádějí v nabídce a tooltipu svou skutečnou cestu.

UI při hoveru pouze čte cache, nevolá filesystem ani systémová API. Názvy jsou
obyčejný Unicode text, nikoli Markdown, HTML, příkazy nebo identita cíle. Nativní
cesty se nikdy nenahrazují zobrazeným textem. Referencí je Salamander
`src/toolbar6.cpp:241` a `src/drivelst.cpp:2561`, `:2621`; jeho synchronní/ANSI
postup se nepřebírá. Podrobný návrh je v
`2026-11-04-compact-tooltips-design.md`.

## 27. Karty, uložené cesty a další upřesnění ovládání

Aktuální produktová verze je **0.1** (`0.1.0` v Cargo SemVer). Titulek nativního
okna je `FM 0.1 — File Manager`; hodnota se odvozuje z verze balíčku.

Každý panel má samostatné karty cest. Výchozí jméno karty vždy odpovídá názvu
aktuální složky a při navigaci se změní; kořen zobrazí svou cestu. Celá cesta je
v tooltipu. Ctrl+T přidá kartu, Ctrl+W zavře aktuální, Ctrl+Tab/Ctrl+Shift+Tab
přepíná karty. Poslední karta zůstává otevřená; limit je32 na panel. V této etapě
jsou karty pouze pro aktuální běh, s vlastní historií, řazením a kurzorem.
Neaktivní karty nedrží velké výpisy ani skeny; přepnutí načte čerstvý obsah.
Opožděné backendové výsledky nesmějí přepsat jinou kartu.

Uložené cesty podle Salamanderu mají jméno, nezměněnou nativní cestu a volitelnou
číselnou zkratku. Nabídka hvězdičky umožní přidání, otevření a odebrání. Až64
záložek,10 číselných slotů; Ctrl+Shift+číslo přiřazuje, Ctrl/Shift+číslo otevírá,
Alt navíc cílí do druhého panelu. Identita není odvozena z názvu ani zobrazené
cesty. Uložené záložky sdílí atomické asynchronní nastavení verze4; nativní
ne-Unicode názvy mají bezeztrátový platformní kodek. Souhrnný limit nastavení
4MiB chrání paměť, není limitem délky cesty; nadlimitní uložení selže bez zkrácení.

Odstraňují se i stavové lišty panelů; běžné úspěšné zprávy nezabírají stálý
footer. Spodní tlačítka mají pro myš základ22px, dotyk40px. Probíhající
velikosti složek ukazují kontrastní barvu a při známém dílčím součtu `≈`, bez `!`.
Options doplňuje systémový font a nezávislé základní velikosti Desktop/Touch
8–32px, výchozí11/15; ostatní text a ovladače se odvozují od těchto hodnot.
Výběr fontů je virtualizovaný, seznam připravuje a cachuje backend.

Ctrl+Enter připojí jméno aktuální položky na konec příkazového editoru;
Ctrl+P vloží cestu aktivního panelu. Použije se bezpečné citování podle shellu,
nikdy automatické spuštění. Salamander zkratky pro implementované funkce,
pixelové rolování s jemným dosednutím na řádek a ikony jednotlivých disků
upřesňuje [podrobný návrh](2026-11-04-navigation-and-appearance-design.md), včetně
testů, ukládání nastavení a omezení této etapy.

Zkratky výběru: Ctrl+A vybrat vše, Ctrl+D zrušit výběr a Ctrl+I invertovat výběr
aktivního panelu. Hromadné výpočty běží v backendu; UI čte sdílený neměnný
bitový snapshot a potvrzuje odpovídající kontext a revizi výběru. Editory si
zachovají běžné Ctrl+A pro text. Úplná aktuální mapa je v
[KEYBOARD_SHORTCUTS.md](../../KEYBOARD_SHORTCUTS.md).

## 28. Plynulý výpočet velikostí a sjednocená navigační lišta

Aktivní výpočet velikostí pokračuje při řazení, práci s výběrem, změně aktivního
panelu i při obnovení stejné cesty. Samostatný identifikátor skenu a původní
nativní cesty oddělují jeho životnost od generace zobrazení. Recyklované číselné
ID nesmí připsat velikost jiné složce. Řazení používá přesný snapshot zobrazený
uživateli; nová ručně spuštěná úloha má vlastní oddělenou cache.

Průběžné překreslení velikostí se slučuje pro oba panely na250ms takt, tedy4×
za sekundu. Ostatní události a přímé ovládání reagují okamžitě. Nehotové součty
mají barvu a `≈`/`…`/`—`; vykřičník se nepoužívá. Dokončený částečný výsledek
si ponechá `≈`, ale běžnou barvou a váhou písma se odliší od aktivního výpočtu.
Řádky souborů ani složek nezobrazují hover tooltipy. Tooltipy ovladačů,
hlaviček, disků a systémových umístění zůstávají dostupné.
Průběžné výsledky ani dokončení výpočtu automaticky nemění pořadí řádků,
ani při zvoleném sloupci Size. Uživatel nové hotové velikosti uplatní ručním
řazením přes záhlaví nebo zkratku. Běžné načtení či obnovení seznamu používá
zvolené řazení.

Rychlá tlačítka disků a umístění se přesunou vedle nabídky disků, bez vlastního
horního řádku. V úzkém panelu se tento pás horizontálně posouvá; základní
navigační ovladače zůstanou přístupné. Podrobnosti jsou v
[aktuálním rozšíření návrhu](2026-11-04-selection-and-continuous-sizes-design.md).

## 29. Systémové ikony, úplná nabídka umístění a příkazová cesta

Rychlá navigační tlačítka a rozbalovací nabídka sdílejí jeden seznam všech
aktuálních disků a speciálních umístění. Domů, Plocha, Dokumenty a Stažené
soubory zůstávají samostatnými volbami i při shodné cílové cestě. Obě místa
poskytují tooltip s úplnou cestou; disky také s dostupným názvem svazku
a volným místem. Změny připojených disků nesmějí zanechat zastaralou volbu.
Identita a revize položky chrání před přesměrováním rozpracovaného kliknutí.

Tlačítka používají odpovídající systémové ikony, pokud je platforma poskytuje.
Windows a Linux mají oddělené adaptéry; načítání a dekódování provádí výhradně
backend s omezenou frontou a cache. Původní barevné pixely se nepřebarvují.
Kde vhodná systémová ikona chybí, zůstává srozumitelná vektorová ikona aplikace.
Ikony a popisky spodních funkčních tlačítek jsou společně vertikálně vystředěné
v obou hustotách ovládání, při zachování rozměrů tlačítek i poměru stran ikon.

Aktivní cesta příkazového řádku se zobrazuje celá, bez výpustky a bez pevného
omezení délky textu. Cesta a editor jsou v jediné vodorovné řadě se shellem,
Options a Run. Cesta má přirozenou šířku, pokud se vejde; u extrémních cest je
omezen pouze vodorovný posuvný výřez, aby editor zůstal použitelný.
Při velkém písmu se minimální výška okna odvodí také od čitelného příkazového řádku
a celého řádku souborů; běžné velikosti zachovají základní minimum 480 px.
Podrobnosti a ověřovací scénáře stanoví
[návrh systémových ovladačů a celé cesty](2026-11-04-native-controls-and-full-path-design.md).

Vkládání cest a jmen do příkazového editoru nepřidává obalující apostrofy.
Jednoduché Unicode cesty zůstanou bez uvozovek, ostatní používají dvojité
uvozovky a přesné escapování daného shellu. PowerShell musí chránit také
číselná jména a všechny podporované typografické uvozovky; POSIX chrání
expanze, zpětná lomítka a substituce. CMD odmítne nejednoznačné procentní
expanze, uvozovky, konce řádků a citovanou cestu končící zpětným lomítkem.
Přesná identita Unicode se ověřuje také skutečnými parsery shellů.
Pravidla vycházejí z [dokumentace PowerShellu](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_quoting_rules),
[POSIX citování v Bash](https://www.gnu.org/software/bash/manual/html_node/Double-Quotes.html)
a [Windows CRT argv pravidel](https://learn.microsoft.com/en-us/cpp/c-language/parsing-c-command-line-arguments?view=msvc-170).

## 30. Hledání psaním, tooltipy a atribut komprese

Prosté psaní v seznamu souborů zahájí hledání začátku názvu. Po dodatečném
upřesnění uživatelem je hledání **vždy Unicode case-insensitive**, také na Linuxu.
Porovnává se původní název; nedochází k normalizaci identity ani odstranění
diakritiky. Hledání je doslovné, bez wildcardů a regulárních výrazů. Zvýrazní se
původní odpovídající prefix, při rozšíření case foldu až po konec původního
grafému. Nativní ne-Unicode jména se nesmějí hledat přes ztrátovou náhradu.

Šipky nahoru/dolů procházejí odpovídající soubory i složky v aktuálním řazení,
bez přetečení přes konec. Escape hledání opustí; Backspace odstraňuje celý
rozšířený grafém a prázdný dotaz režim ukončí. `..` zůstává navigací a nehledá se.
IME preedit je lokální; backend dostane až potvrzený text. Ctrl+Space patří IME,
Ctrl+P nadále vkládá cestu. Windows Shift+písmeno disku má přednost; Shift+číslo
pro uložené cesty je při hledání/kompozici dočasně potlačeno. Ostatní editory
zachovávají běžné chování.

Options nabízí filtrování při psaní: zobrazí jen shodné položky namísto skákání
v celém seznamu. Volba je ve striktních nastaveních verze4 jako
`quick_search_filter`, výchozí `false`; verze1–3 se migrují s vypnutým filtrem.
Předčasné místní změny přežijí pozdní načtení uložených nastavení. Dotaz je
samostatný pro každý panel, přežívá řazení a obnovu stejné složky; změna složky
nebo karty jej ruší. Prostor cesty slouží také pro kontextový dotaz bez trvalého
nového řádku UI.

Veškeré porovnávání a vytváření indexu probíhá na backendu nad neměnným úplným
snapshotem. Dva omezené, samostatné proudy přijímají poslední dotaz panelu;
starší práci kooperativně ruší a kontrolují generaci navigace, přesnou identitu
zdrojového Arc a revizi dotazu i po čekání na výstupní frontu. UI ověření opakuje.
Filtrování nemění kanonický obsah pro watchery a výpočet velikostí. Hromadný
výběr a vlastnosti používají skutečně zobrazenou projekci; skryté položky nejsou
akčními cíli. Klávesová aktivace během čekání nebo bez shod nesmí otevřít
zastaralou neodpovídající položku. Velká data se uvolňují na backendu.

Tooltipy mají menší odvozené písmo. Preferované umístění je vpravo dole od
kurzoru s mezerou přibližně14/18px; u hran se jednotlivé osy obrátí a bublina
zůstane ve viditelné ploše s8px okrajem. Velké bubliny se přizpůsobí dostupnému
prostoru, aby nezakryly kurzor a klikací místo. Zachová se úplný přístupný text,
průchod kliknutí a nativní životní cyklus i uvnitř popupů. Tooltipy na řádcích
souborů zůstávají odstraněné.

Soubor nebo složka s kompresí hlášenou filesystémem má malou značku u nativní
ikony. `EntryMetadata.compression` rozlišuje `Unknown`, `Uncompressed` a
`Compressed`; přípona archivu ani řídká alokace nejsou důkazem komprese.
Windows adaptér čte nativní atribut, Linux samostatně používá bezpečné `statx`
s kontrolou masky podpory. Metadata se načítají na backendu bez otevírání obsahu
a bez následování koncového odkazu. Zobrazená velikost zůstává logická.

Odkazy na složky a Windows junctiony mají rozpoznatelnou ikonu odkazu. Windows
načte systémový překryv na základní ikoně složky/souboru; ostatní platformy použijí
vhodnou systémovou ikonu, případně vlastní značku. Typ `DirectoryLink` zůstává
oddělený od skutečné složky: rekurzivní výpočet velikostí jej nesmí následovat.
Windows rozpozná adresářový odkaz z údajů enumerace bez dotazu na cíl; na Linuxu
se kvůli ikoně neprovádí implicitní průchod či síťový dotaz na cíl symlinku.
Načítání a skládání ikon probíhá na backendu. Překryv odkazu může současně
obsahovat označení komprese. Koncový odkaz se sleduje až při výslovné navigaci.

Podrobná rozhraní, prior art Salamanderu a ověřovací scénáře jsou v
[návrhu hledání, tooltipů a metadat](2026-11-05-panel-search-and-metadata-design.md).
[README](../../../README.md) obsahuje sestavení a spuštění Windows x64 a
Linuxu/Ubuntu/Mintu včetně `build.bat` a `build.sh`; macOS zůstává budoucí etapou.

## 31. Sdílení kódu a velikost generovaného Rustu

Opakované chování patří do malých sdílených funkcí; opakované ovládací prvky do
komponent s daty a callbacky. Platformní implementace zůstávají oddělené, společná
je pouze neutrální logika. Pravidlo platí také pro vstupy Slint kompilátoru:
tooltip vložený jako dítě každé instance může vynutit vložení celé komponenty.
Sdílené obaly editorů a ComboBoxů proto vlastní tooltip uvnitř své definice.
Nabídky F9/F10 sdílejí tělo a panelové klávesy zpracovává společný Rust router.

Izolovaný tooltip adaptér ukládá původní tělo vytvoření tooltipu jednou do privátní
funkce a opakovaně ji volá. Výstup se standardně formátuje jako Rust AST; ruční
úpravy vygenerovaného textu ani zdrojů závislostí se nepoužívají. Při změnách
komponent se kontroluje skutečný generovaný objem při stejném vstupu a zároveň
chování, Unicode, focus a vykreslení. Velikost zdrojového výstupu sama neurčuje
velikost binárky ani spotřebu paměti. Podrobnosti měření a hranic sdílení uvádí
[rozšíření návrhu](2026-11-05-panel-search-and-metadata-design.md#reuse-and-generated-rust).

Podrobný audit rozlišuje vlastní Slint zdroje od výsledku překladu. V prvním
optimalizovaném checkpointu má dvanáct zdrojů dohromady přibližně 125 KB; generovaný
Rust má 5,96 MB normalizovaných tokenů a 10,59 MB po formátování. Přibližně 59 % tohoto
výpisu tvoří inicializace widgetů a vazeb. FilePane i TipBody už mají jednu sdílenou
implementaci; další celý panel přidá v diagnostickém příkladu pouze 74 KB. Přidávání
dalších pomocných funkcí samo o sobě zbývající megabajty neodstraní.

Následné sdílení tlačítek snížilo výstup na 4,87 MB normalizovaných tokenů a
8,70 MB po formátování, celkem o 40,6 % proti původnímu adaptérem generovanému
výstupu. Počet implementací komponent klesl ze 130 na 92. Zbývající inicializace
představují 60,7 % formátovaného výpisu; tento výsledek není tvrzením o dosažení
optima ani o velikosti binárky či spotřebě RAM.

Další postup je omezovat varianty složených ovladačů, sdílet dialogová těla pomocí
modelů a nepřidávat Tooltip jako dítě každého tlačítka. Stejný princip obalu s
vlastností `hint` platí pro tlačítka, ComboBoxy i editory. Výraznější úspory přes
vlastní lehké ovladače nebo sdílenou reprezentaci popupů se musí prokázat úplným
prototypem včetně klávesnice, IME, přístupnosti a systémových ikon. Nepředpokládá
se neověřené procento úspory ani úplná aplikace ve stovkách kilobajtů.
Čísla, omezení a další možnosti dokumentuje
[rozbor generování UI](../../verification/slint-code-generation-size.md).
