# Midnight Explorer — dvoupanelový souborový manažer v Rustu

Datum: 4. listopadu 2026; doplněno 7. listopadu 2026

Stav: průběžný návrh produktu Midnight Explorer 0.1. Aktuální rozšíření operací,
ovládání a pluginů níže mají přednost před historickými etapami tohoto dokumentu.
Pořadí přijatých navazujících implementací zachycuje
[plán etap](../plans/2026-11-05-follow-up-stages.md); samotný návrh neznamená hotovou funkci.

[Nový prázdný soubor](2026-11-05-empty-file-design.md): tlačítko v panelu a Shift+F4,
dialog názvu, výhradní vytvoření na backendu bez přepsání existujícího souboru.

Viewer etapa: implementované [veřejné API/SDK](../../../api/README.md), bezpečnostní
kontroly loaderu, sdílený Unicode renderer a samostatné pluginy text/hex, source
a Markdown. F3 vybírá plugin podle přípony, Alt+F3 žádá základní textový režim.
Pluginy zůstávají v jednom Git repozitáři, každý ve vlastním projektu/testech.
Aktuální hranice a testy popisuje [ověření viewerů](../../verification/viewer-implementation.md)
a [developer dokumentace API](../../developer/README.md). Filesystem pluginy
a migrace nativního filesystemu zůstávají navazující etapou.

Aktuální drobná rozšíření: [unikátní číslované Hot Paths](2026-11-06-unique-numbered-hot-paths.md)
se správou zkratek bez zavření dialogu; [souhrn potvrzení mazání](2026-11-06-delete-summary.md)
s počty a součtem velikostí místo seznamu všech souborů. Light a Dark nyní stejně
jako ostatní schémata barevně odlišují složky; pravidla kontrastu jsou v
[návrhu témat](2026-11-06-theme-expansion-design.md). Výběr knihoven pro budoucí
obrázkový viewer a mediální náhledy upřesňuje
[návrh thumbnailů](2026-11-05-panel-views-thumbnails-design.md).

Další implementované úpravy: [F7 pro absolutní i relativní cesty](2026-11-06-create-any-path.md),
[aktivace panelu kliknutím](2026-11-06-panel-activation.md),
[přehlednější atributy a časy](2026-11-06-metadata-dialog-refinement.md)
a [sjednocené zobrazení chyb](2026-11-06-error-presentation-design.md).

[Kontrolní součty na Ctrl+F8](2026-11-06-checksums-design.md) mají backend a
samostatné nemodální okno: MD5/SHA-1 pro kompatibilitu, výchozí SHA-256 a volitelné
SHA-512, zachycený výběr, neblokující průběh, kopírování hashů a bezpečný export.
Přesný stav ověření uvádí [záznam testů](../../verification/2026-11-07-checksum-window.md).
Ctrl+F7 zůstává změně velikosti písmen názvu. Ověřování kopie nadále používá SHA-256.

[Dialog kopírování F5](2026-11-07-copy-dialog-design.md) zachytí výběr a cestu
druhého panelu, umožní upravit cílovou složku a zařadí úlohu přímo do společné
ověřované fronty. Nepoužívá systémovou schránku. Výslovné Merge/Merge all a
obnovitelné Replace/Replace all doplňují nativní transferové schopnosti;
nahrazený originál zůstává v recovery složce. F6 a Cut/Paste sdílejí potvrzovací
dialog a ověřovanou frontu pro omezený obnovitelný přesun běžných souborů.
Rozsah, limity a zbývající ověření určuje [transferový kontrakt](../../developer/TRANSFERS.md).

## 1. Záměr a potvrzená rozhodnutí

Vznikne svižný desktopový souborový manažer inspirovaný Altap Salamanderem a Total Commanderem. Začne malým, použitelným základem a bude se rozšiřovat po samostatně ověřitelných etapách.

Požadavky uživatele:

- Rust, UI ve Slintu, Tokio pro asynchronní orchestrace.
- Všechny knihovny třetích stran musejí umožnit zamýšlený vývoj, použití a distribuci
  bez povinných poplatků či royalties; preferujeme open source. Platí i pro
  transitivní závislosti, API/SDK, pluginy, build a testy. Licence a povinnosti
  eviduje [licenční politika](../../DEPENDENCY_POLICY.md) a
  [úplný seznam knihoven](../../THIRD_PARTY.md).
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
- Aktuální etapu rozšiřuje [návrh operací a ovládání](2026-11-05-rename-delete-scroll-design.md): rename/delete/F7, fronta, F11/F12, editace cesty, trvalé karty, locale a režimy panelů/náhledy. Má přednost před staršími read-only omezeními níže.
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
| `midnight-core` | Doménové typy, identifikátory, příkazy/události, chyby, rozhraní providerů a jejich schopnosti | Žádné |
| `midnight-engine` | Navigace, načítání adresářů, plánování, kopírování, ověřování a stav úloh; Tokio | `midnight-core` |
| `midnight-platform` | Malá továrna/reexport podle targetu | `midnight-core` a zvolený platformní crate |
| `midnight-platform-common` | Přenositelné bezpečné Rust operace a společné pomocné struktury | `midnight-core` |
| `midnight-platform-windows` | Windows implementace služeb, Win32, svazky a Windows specifika | `midnight-core`, `midnight-platform-common` |
| `midnight-platform-linux` | Linux implementace služeb, mountinfo a Linux specifika | `midnight-core`, `midnight-platform-common` |
| `midnight-ui` | `.slint` soubory, klávesové ovládání, presenter, modely a převod dat pro zobrazení | `midnight-core` |
| `midnight-explorer` | Spuštění, konfigurace, vytvoření runtime, propojení zpráv, injektování providerů a ukončení | Ostatní crates |

`midnight-core`, `midnight-engine` a `midnight-platform` neznají Slint. `midnight-ui` nevolá filesystem ani konkrétní engine a není závislé na jeho implementaci. `midnight-explorer` dodá presenteru neblokující vstupní rozhraní a propojí výstupní události s UI. Rozhraní providerů budou odpovídat potřebám aktuálních etap; nevznikne předem rozsáhlá abstrakce všech souborových systémů.

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
- Seznam se sloupci název, velikost a datum změny; adresáře jsou standardně první.
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

### Implementovaný první řez COPY-03 (2026-11-07)

Existující API zůstávají `ContentOnly`. Výslovné `RequireModifiedTime` podporuje
nejvýše 64 různých nativních běžných souborů na Windows/Linuxu; složky, odkazy a
virtuální zdroje odmítá bez tichého snížení záruky. UI volba zachování přesného
času změny je ve výchozím stavu vypnutá; její testy UI prošly na Windows i Linuxu
a aktuální Windows release prošel nativním ověřením spuštění.
[Ověření](../../verification/2026-11-07-metadata-filters-provider-checkpoint.md),
[kontrakt](../../developer/TRANSFERS.md) a
[volba dialogu](../../developer/TRANSFER_OPTIONS.md) vymezují skutečný rozsah.

Backend zachytí čas ze zdrojového handle před čtením dat. Po zdrojovém SHA-256 a
úplném nezávislém čtení cíle nastaví čas na vlastním neveřejném souboru, provede
synchronizaci a přesný metadata readback. Typy a schopnosti celé dávky se ověřují
před zápisem dat; konkrétní nepodporované rozlišení cíle se může zjistit až po
ověření dat, před publikováním daného souboru. Dřívější dokončené cíle zůstanou;
chyba před záchyty blokuje všechny zdrojové záchyty přesunu. Časové kontroly pouze
doplňují úplné hashe a nové nativní revize; publikovaný cíl se nikdy neopravuje,
aby se skryla změna. Původní data přesunu zůstávají v recovery, místo se neuvolňuje.

Ostatní časy, atributy, práva, ACL, xattrs, proudy, metadata adresářů a zachování
odkazů tím nejsou dokončeny. Obecný přesun adresářů zůstává samostatnou etapou.

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

Platformní vrstva se skládá z malé továrny/fasády `midnight-platform`, přenositelného
`midnight-platform-common` a samostatných `midnight-platform-windows`, `midnight-platform-linux`,
později `midnight-platform-macos`. Továrna pouze vybere/reexportuje implementaci pro
daný target; neobsahuje implementace filesystemu. Windows Win32 API a Linux
mountinfo/native logika se nikdy nesdílejí v jednom implementačním souboru.

Společný kód obsahuje jen skutečně přenositelné operace přes bezpečné Rust API,
pomocné datové struktury a doménové smlouvy. OS-specifické testy patří také do
samostatných souborů; stejné smluvní testy běží nad zvolenou implementací na každé
podporované platformě. Platformní crate se nekompiluje jako cizí implementace na
jiném targetu. UI ani engine neimportují Win32, `/proc`, konkrétní platformní
crate nebo `cfg` větve s OS logikou.

### Povinné služby první etapy

Ve `midnight-core` jsou objektově použitelná rozhraní:

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
na `midnight-platform-common`. Podpisy zachovávají bezeztrátové cesty, strukturované
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
| Space / Insert | Multivýběr a posun kurzoru; Space navíc počítá velikost skutečné složky na pozadí |
| Alt+Left / Alt+Right, Ctrl+R | Historie / nový výpis aktivního umístění |
| Kliknutí na záhlaví Name/Size/Modified | První volba sloupce vzestupně; opakované kliknutí přepíná asc/desc; šipka ukazuje směr |
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

Scroll zachovává nativní dojezd prstu/touchpadu. Diskrétní kolečko v souborovém
panelu převádí na krátkou pixelovou animaci a po klidu zarovná řádek. Kliknutí,
klávesnice či změna cíle zruší zbylý pohyb; metadata nevracejí viewport ke kurzoru.

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

Horní branding řádek Midnight Explorer/About se odstraní. Tlačítko **Options** bude u příkazového
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
Pluginy jsou v jednom Git repozitáři, ale mají samostatné Cargo projekty,
buildy a vlastní testy/CI. Sdílí pouze veřejné
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

Implementační doplněk k oddílu 25: [nativní transfer kontrakt](../../developer/TRANSFERS.md)
odděluje výslovné „Replace · keep original“ od silné podmíněné výměny identity.
Původní objekty zůstávají v soukromém recovery adresáři; částečný commit nikdy
neopravňuje smazat zdroj. Merge a Replace all mají nezávislý rozsah jediné úlohy.
Nově vzniklý či změněný cíl vyžaduje nové potvrzení i po Replace all. Dialog
ukazuje typ, úplnou cestu, bajty a zachycený UTC čas; ve větším okně porovnává
zdroj a cíl vedle sebe. Jobs poskytuje omezené dočasné podrobnosti obnovy, nikoli
trvalý katalog. F6 a úplné zachování metadat mají samostatné navazující kontrakty.

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
okna je `Midnight Explorer 0.1 — File Manager`; hodnota se odvozuje z verze balíčku.

Každý panel má samostatné karty cest. Výchozí jméno karty vždy odpovídá názvu
aktuální složky a při navigaci se změní; kořen zobrazí svou cestu. Celá cesta je
v tooltipu. Ctrl+T přidá kartu, Ctrl+W zavře aktuální, Ctrl+Shift+Tab volí předchozí,
Ctrl+Shift+1…9 konkrétní kartu. Ctrl+Tab fokusuje příkazový řádek. Poslední karta
zůstává otevřená; limit je 32 na panel. Karty a aktivní pozice se ukládají mezi běhy.
Levý dvojklik do volného místa za kartami přidá kartu se stejným aktuálním umístěním
daného panelu. Dvojklik na existující kartu, pravé a prostřední tlačítko další kartu
nevytvářejí; prostřední tlačítko zachovává zavření karty.
Každá karta má vlastní historii, řazení a kurzor pro aktuální běh.
Neaktivní karty nedrží velké výpisy ani skeny; přepnutí načte čerstvý obsah.
Opožděné backendové výsledky nesmějí přepsat jinou kartu.

Uložené cesty podle Salamanderu mají jméno, nezměněnou nativní cestu a volitelnou
číselnou zkratku. Nabídka hvězdičky umožní přidání, otevření a odebrání. Až64
záložek,10 číselných slotů; správce přiřazuje, Ctrl/Shift+číslo otevírá,
Alt navíc cílí do druhého panelu. Identita není odvozena z názvu ani zobrazené
cesty. Uložené záložky sdílí atomické asynchronní nastavení verze5; nativní
ne-Unicode názvy mají bezeztrátový platformní kodek. Souhrnný limit nastavení
4MiB chrání paměť, není limitem délky cesty; nadlimitní uložení selže bez zkrácení.

Běžné úspěšné zprávy nezabírají stálý footer. Každý panel má nově kompaktní
metadata řádku pod kurzorem, kopírovatelná kliknutím. Spodní tlačítka mají pro myš základ22px, dotyk40px. Probíhající
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
Od revize 2026-11-07 je minimum přesně 320×200 logických pixelů pro obě hustoty
a všechny podporované velikosti písma; preferovaná velikost zůstává 1340×820.
Při nedostatku prostoru se zobrazí aktivní panel, lišta pro přepnutí druhého
panelu, cesta, soubory a příkazový řádek. Pomocné lišty se obnoví po zvětšení.
Funkční tlačítka lze vodorovně posouvat, zatímco Options a Jobs zůstávají viditelné.
Popisky všech Ctrl+F1–F12 vycházejí z aktuálního nastavení; nepřiřazená tlačítka
zobrazí klávesu a jsou zakázaná. Rodičovský řádek `..` není cílem souborových
operací; skutečný výběr dovoluje Copy/Delete i při zaměření rodiče.
Podrobnosti malého okna a jeho testů jsou v [ověření](../../verification/small-window.md).
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

Opakované chování patří do sdílených funkcí a komponent s daty a callbacky.
Platformní implementace zůstávají oddělené. Tooltipy vlastní sdílené obaly;
vložené dítě v každé instanci by mohlo vynutit duplikaci komponenty. Generovaný
Rust se ručně nemění. Každé zmenšení musí zachovat Unicode, IME, focus,
přístupnost a vykreslení. Velikost Rust výpisu není velikost binárky ani RAM.

Předchozí optimalizace dosáhla 4,87 MB normalizovaných tokenů / 8,70 MB
formátovaného Rustu, o40,6 % méně než původní adaptér. Nejde o odhad nových
komponent ani tvrzení o optimu. Měření a další možnosti jsou v
[rozboru generování UI](../../verification/slint-code-generation-size.md).

## 32. Detailní navigace, historie a síťový provider

Kontrakty cest, historie posledních 100 unikátních příkazů, detailních lišt,
Space/Insert, Hot Paths, Options a výstupů pod `_build` jsou soustředěné v
[rozšíření panelů](2026-11-05-panel-navigation-details-design.md).
Novější [trvalé karty a nastavení v6](2026-11-05-tab-session-extension.md) přidávají
obnovu karet, režimů panelů a bezpečnou migraci nastavení při přejmenování produktu.
[Síťový filesystem](2026-11-05-network-filesystem-design.md) bude samostatný
API-only plugin pro dostupná zařízení/služby/sdílení. Zjišťování celé sítě zatím
není implementované; dostupnost nelze zaručit pro zařízení blokující discovery.

[Nové ovládání a layout](2026-11-05-panel-interaction-layout-design.md) definuje
Ctrl+Tab/Ctrl+U, splitter, karty, výběr, terminál, atributy, Light a F3/Alt+F3.

## 33. Nový název a navazující implementace

Produkt se jmenuje **Midnight Explorer**; release používá optimalizaci 3, fat LTO
a jednu codegen jednotku, s kontrolami přetečení a obnovou chyb workerů.
Podrobnosti jsou v [release konfiguraci](../../RELEASE_BUILD.md).
[Viewer rozšíření](2026-11-05-viewer-implementation-extension.md) nyní požaduje
tři samostatné pluginy, Escape/Space, velikost okna podle aplikace a externí editor F4.
Následuje ověřené dávkové copy/move, změna atributů/časů a nemodální hledání podle
[přijatého pořadí etap](../plans/2026-11-05-follow-up-stages.md).

## 34. Systémové nabídky, schránka a nastavitelné zkratky

[Rozšíření ovládání panelů](2026-11-06-panel-shell-design.md) přidává systémové
nabídky souborů a disků, Ctrl+C/Ctrl+V, ověřené kopírování s frontou, součet bajtů
vybraných souborů, volné místo, řazení ve všech režimech a oddělené akce Hot Paths.
Dvojklik zachovává identitu buňky i během překreslení. Ctrl+Enter vloží název
souboru a zaměří příkazový řádek. SHA-256 porovnává čtení zdroje s nezávislým
čtením zapsaného cíle; náhrada existujícího cíle a Move zatím zůstávají nepodporované.

[Nastavitelné klávesové zkratky](2026-11-06-configurable-shortcuts-design.md)
používají společný katalog akcí, kontexty, kontrolu kolizí, ukládání v Options
a obnovení výchozích hodnot. Standardní editace textu, IME a klávesy rezervované
operačním systémem zachovávají své nativní chování.

## 35. Řazení podle systémového locale

[Rozšíření řazení](2026-11-06-locale-sorting-design.md) používá nativní váhy názvů
a přípon včetně velikosti písmen a diakritiky. Shodné velikosti a časy se řadí
stejným pravidlem podle názvu. Locale se zachytí do restartu aplikace; Linux při
nedostupném locale použije C. Neplatné nativní kódování se nikdy nenahrazuje ani
nezkracuje: používá deterministické pořadí původních názvů. Klíče vznikají na
backendu, mají limit 4 MiB na klíč a zůstávají v paměti pro další řazení.
[Implementační plán](../plans/2026-11-06-locale-sorting.md) popisuje testy a
oddělené adaptéry Windows/Linux.

## 36. Osm témat a kompaktní příkazový řádek

[Rozšíření vzhledu](2026-11-06-theme-expansion-design.md) zachovává Blue, Dark
a Light a přidává Dracula, Nord, Gruvbox Dark, Catppuccin Mocha a Solarized Light.
Palety sdílejí sémantické barvy všech komponent; systémové ikony se nepřebarvují.
Nové palety mají testovaný kontrast textu nejméně 4,5:1 a zvýrazněných hran 3:1.
Volba se ukládá stabilním názvem. Editor příkazů, jeho cesta, historie i tlačítka
mají shodnou výšku se spodní funkční lištou, při zachování Desktop/Touch a fontu.

## 37. Porovnání, filtry, Ctrl lišta a editor metadat

[Návrh Commander nástrojů](2026-11-06-commander-tools-design.md) rozpracovává
nově zadané funkce podle zdrojů Salamanderu. Alt+nahoru/dolů v příkazovém editoru
cyklicky prochází uložených nejvýše 100 unikátních příkazů a nemění typ terminálu.
Enter příkaz spustí. Držení Ctrl přepne spodní lištu podle skutečně nastavených
vazeb Ctrl+F1 až Ctrl+F12. Uvolnění klávesy ani změna focusu nesmějí změnit akci
už stisknutého tlačítka; AltGr tuto vrstvu nezapíná.

Ctrl+F10 porovnává viditelné adresáře na backendu podle velikosti a změnového
času s volitelnou tolerancí (výchozí dvě sekundy). Označuje chybějící, větší nebo
novější soubory. Shoda metadat neznamená shodu obsahu. Rozšíření o porovnání
obsahu a rekurzi zůstává další etapou. Zrušený nebo zastaralý výsledek nemění
výběr. Ctrl+F12 nastavuje samostatný uložený filtr panelu: Masks/Regex ponechávají
složky viditelné, Expression vyhodnocuje typovaná metadata i u složek. Ctrl+F11 zvětšuje aktivní panel, ponechá
aktivační pruh druhého a při vypnutí obnoví původní poměr šířek.

Ctrl+F2 otevírá společný editor atributů, časů a velikosti písmen; Ctrl+F7 tentýž
editor se zapnutou změnou písmen. Výchozí stav ostatních polí je „beze změny“.
Náhled i provedení mají identitu operace a revizi návrhu, aby starší náhled nemohl
potvrdit jiné změny. Windows a Linux mají samostatné nativní implementace a své
schopnosti. Časy vyžadují explicitní časové pásmo; „aktuální čas“ zachytí backend
jednou pro celou dávku. Unicode převod zachovává původní nativní části názvu,
nikdy neodvozuje název ze ztrátového zobrazovacího řetězce. Rekurze nenásleduje
odkazy a zpracuje potomky před rodičem. Kolize nesmí přepsat jiný soubor.
Podrobný návrh stanoví limity zdrojů, částečné výsledky a testy nativních operací.

## 38. Přetahování souborů a zpřesnění vzhledu

[Návrh drag-and-drop](2026-11-06-file-drag-drop-design.md) přidává kopírování mezi
panely a předávání souborů jiným aplikacím. Příjem v aplikaci používá stejnou frontu
a nezávislé ověření SHA-256 jako schránka. Nativní transport má samostatné adaptéry
Windows a Linux/X11, omezené fronty a zachovává původní Unicode/nativní názvy.
Přetažení ven provádí cílová aplikace. Výchozí akce nepřesouvá ani nemaže zdroj.

Horní lišta má stejný odstup od disků jako disky od cesty. Dlouhá cesta zůstává
viditelná od konce, s výpustkou vlevo a možností otevřít celý editor. Viditelné
části zachovávají původní tokeny pro navigaci i kopírování. Všech osm témat má
výraznější rozlišení ploch, výběru a okrajů; kontrast se kontroluje společným testem.

## 39. Podporované systémy a minimální prostředky

Závazný přehled cílových verzí OS, architektur, knihoven a ověření obsahují
[systémové požadavky](../../SYSTEM_REQUIREMENTS.md). Pro verzi 0.1 je cílem
Windows 10 22H2 / Windows 11 x64 s MSVC, Ubuntu od 24.04 LTS a Mint 22.x x86-64.
macOS nemá implementovaný adaptér, proto zatím nemá podporovanou verzi ani
ověřené minimum RAM. Nová platforma musí projít vlastní kvalifikací.

Výslovné linuxové cíle zahrnují také udržované stabilní Fedory od verze 44 a openSUSE Leap 16.x
i Tumbleweed. Používají stejný linuxový adaptér; rozdíly patří do balíčkování,
závislostí a detekce schopností, nikoli do kopie implementace filesystemu pro každou
distribuci. README uvádí `dnf` a `zypper`. Nativní testy Fedory/openSUSE zatím nejsou
provedené; ověření musí pokrýt X11/Wayland, běžná bezpečnostní pravidla, Btrfs/ext4
a příslušné grafické ovladače. U Tumbleweed zaznamenáváme přesný snapshot. SUSE
Linux Enterprise potřebuje samostatně určenou a ověřenou verzi.

Pro běžný provoz plánujeme alespoň 4 GB celkové RAM (Ubuntu Desktop 26.04: 6 GB),
doporučeně 8 GB, a dostupnou rezervu 1 GiB pro aplikaci; pro velké adresáře
a souběžné úlohy alespoň 2 GiB. Jde o plánovací hodnoty, nikoli naměřenou dolní
mez nebo pevný limit procesu. Kopírování a checksumy jsou proudové a nesmějí
alokovat celý soubor. Testy na omezené RAM musí před vydáním ověřit velké výpisy,
ikony, hledání, více úloh a odezvu UI. Nároky pluginů se uvádějí samostatně.

Pro kompilaci doporučujeme 16 GB RAM, pro souběžné sestavení a fat LTO 32 GB.
Varianta 8 GB s jednou Cargo úlohou není zatím ověřené minimum. Vývojové cache
pro více platforem a profilů mohou zabrat přes 100 GiB; veškeré sestavené a dočasné
výstupy zůstávají pod `_build`. Distribuovaný Linux build používá nejstarší cílový
userspace Ubuntu 24.04 / glibc 2.39. Bezpečné nativní operace mají navíc konkrétní
požadavky na kernel a schopnosti souborového systému; neúspěch nikdy nevede
k vypnutí kontrol identity nebo ověření kopie.

Ověření Unicode musí kontrolovat skutečný renderer a viditelné glyphy. Na virtuálním
Linux displeji může Slint bez vhodného GPU přejít na softwarový renderer, který
nezobrazuje bitmapové emoji. QA může pro ověření WGPU použít `SLINT_WGPU_CPU=1`
a testovací fonty; takový výsledek není kvalifikací fyzického GPU ani výkonu.

## 40. Ikona aplikace

Ikonou Midnight Exploreru je měsíc se světlou stříbrnou a tmavou modrošedou
polovinou na průhledném pozadí. Okno používá vložený PNG zdroj; Windows EXE
obsahuje ICO s velikostmi 16 až 256 px pro Průzkumník, titulkový pruh a hlavní
panel. Systémové ikony souborů a disků tím nejsou nahrazeny. Zdrojový obrázek,
exporty a popis jejich původu jsou v `assets/branding`; běžná kompilace nepotřebuje
obrázkovou službu ani nástroje pro převod grafiky. Windows resource compiler běží
pouze při sestavení a jeho výstup zůstává v Cargo `OUT_DIR` pod `_build`.
Wayland používá shodný XDG identifikátor aplikace, desktopový soubor a nainstalovanou
PNG ikonu; samotná vlastnost ikony okna tam nestačí. Postup instalace je v README.

## 41. Přehlednější nastavení, klávesové zkratky a úlohy

Nastavení používá větší okno omezené rozměry hlavního okna, s tématy vlevo,
posuvným obsahem a pevnou spodní řadou akcí. Rozměry respektují desktop/touch
profil a zvolené písmo. Kategorie lze přepínat myší i klávesnicí; změna kategorie
šipkami musí přesunout také fokus.

Editor zkratek vychází z dodané ukázky Visual Studia: vyhledávání podle názvu
příkazu nebo ID, virtualizovaný seznam příkazů, výběr existující zkratky a záznam
nové skutečným stiskem kláves. Assign přidává alias, Remove odstraní pouze zvolený
alias. Před přiřazením se zobrazí kontext a aktuální vlastníci zkratky; konflikt
v překrývajícím se kontextu neprovede částečnou změnu. Záznam kláves nikdy
nespouští příkazy aplikace. Tab/Escape zaznamená klávesu a opustí záznamové pole.
Obnova celého výchozího mapování vyžaduje potvrzení v editoru. Kontext určuje
příkaz a pravidla pro editaci textu; nabízíme pouze skutečně implementované
kontexty a mapování. Podrobnosti: [návrh editoru](2026-11-07-keyboard-options-design.md).

Jobs používá virtualizovaný seznam operací a samostatný detail vybrané úlohy.
Výběr je určen stabilním ID, takže jej nezmění průběžná aktualizace ani nová
úloha. Zobrazuje ikonu, stav, fázi, úplné cesty a známé počty/bajty. Kopie bez
známého celkového objemu neukazuje odhadnutá procenta. Skutečná chyba má výraznou
ikonu; čekání na rozhodnutí není chyba. Cancel, Resolve, Copy details a Close
jsou ve spodní řadě; kopírování detailu používá backend schránky. Zavření okna
ponechá úlohy běžet a stisk akce sám okno nezavírá.

Nabídka disků obsahuje navíc Pictures, Videos, Music, uživatelský TEMP a dostupné
osobní/pracovní účty OneDrive. Windows cesty pocházejí z Known Folders a registrací
účtů, takže respektují přesměrování. Linux používá XDG user directories a své
dočasné umístění. Neexistující složky nevytváříme; všechny dotazy a ikony zajišťuje
backend příslušné platformy. Podrobnosti: [umístění a Jobs](2026-11-07-locations-and-jobs-design.md).

Pokud jsou ve výběru složky s dokončeným bezchybným měřením, stavový řádek ukáže
i jejich známý součet. Částečné nebo chybějící výsledky musí zůstat rozeznatelné
od konečné velikosti. Součty zpracovává backend pro zachycený výběr a verzi výpisu;
UI nesmí procházet velký výběr při každém překreslení.

## 42. Samostatné archivní a mediální pluginy

Ve stejném Git repozitáři zůstávají nezávislé Cargo projekty: veřejné protokoly
`filesystem-api`, `thumbnail-api` a `worker-api` a samostatné implementace archivů,
obrázků a videí. Pluginy mají vlastní testy, závisí pouze na verzovaném veřejném
API a nemění zdrojové soubory hostitele. Nasazují se vedle programu do
`filesystems/` a `thumbnailers/`; chybějící plugin nezpřístupňuje danou funkci.
Dosavadní vydané viewer API/SDK 0.1.0 se tím nemění.

Archivní provider nabízí čtení ZIP, 7z, GZIP, TAR a TAR.GZ, navigaci, hledání,
řazení a F3 nad zachyceným obsahem. Identitu člena netvoří lokální cesta. Záložky
uchovávají typovaný popis provideru, zdrojového archivu a nezměněné komponenty;
po změně nebo ztrátě zdroje zůstanou výslovně nedostupné. První implementace má
rozpočty 64 MiB pro archiv/člena a 128 MiB pro dekódovaný součet. Neprovádí zápis
do archivů, vnořené mounty, dešifrování ani spuštění člena externí aplikací. F5
vykopírování do nativního cíle doplňuje navazující etapa z 2026-11-07.
Tyto rozpočty neomezují délku nativních cest. Přesné smlouvy a další limity uvádí
[filesystem API](../../developer/FILESYSTEM_API.md).

### Rozpoznávání a volitelné asociace archivů

Hostitel sdílí jednu tabulku 67 přípon (51 ZIP kontejnerů a šestnáct přípon ostatních
dekodérů). Seznam a zdroje jsou v [archivních asociacích](../../ARCHIVE_FORMATS.md).
Options → Archives nabízí každou příponu se zaškrtávátkem a ukládá jednotlivé
uživatelské odchylky od výchozího stavu. ZIP a vývojářské balíčky WHL, EGG, WAR,
EAR, AAR, AAB, NUPKG a SNUPKG jsou zapnuté. Dokumenty Office/OpenDocument, knihy,
komiksy, mapy, obrazové projekty, spustitelné JAR/PYZ/PYZW a APK/VSIX/XPI zůstávají
v seznamu vypnuté, aby Enter běžně otevřel systémovou aplikaci. Uživatel je může
jednotlivě zapnout. Nejde o změnu asociací operačního systému.

Backend při výpisu připraví kandidátní typ bez čtení obsahu. Značka vstupní šipky
u systémové ikony se vykreslí ve všech třech režimech pouze pro nativní soubor se
zapnutou asociací a skutečně dostupným providerem daného formátu. Přítomnost
provideru jiného formátu nestačí. Pozdní dokončení discovery či změna nastavení
aktualizuje značky; metadata komprese filesystemu a systémové link overlay zůstávají
samostatné. Ikona znamená dostupný handler, nikoli předem ověřený obsah souboru.
Plugin validuje obsah až při otevření na backendu. Vnořené archivy ani adresáře
s archivní příponou nedostávají značku nepodporovaného vstupu.

Provider umí F3 čtení a F5 vykopírování do nativního cíle přes stávající frontu,
bezpečnou přípravu cíle, konfliktní dialog, průběh a SHA-256 kontrolu nezávislým
čtením cíle před zveřejněním. Zápis/přesun uvnitř archivů zůstává budoucí etapou.

Obrazové a video náhledy vznikají v omezených pomocných procesech, na Windows
v AppContaineru s Job Objectem a na Linuxu se seccomp a limity prostředků.
Hostitel zprostředkuje pouze čtení zachyceného obsahu; dekodér nemá přístup
k libovolným souborům ani síti. Nejvýše dvě dekódovací úlohy sdílejí omezenou
frontu a rozpočet paměti. Přístup ke zdrojovým datům používá společný rozpočet
s měřením složek a kopírováním: souběh dovolují pouze potvrzená lokální SSD,
HDD/síť/neznámé úložiště mají sériový přístup po omezených blocích.

Každý úspěšný snímek se nejprve bezpečně uloží do soukromé diskové cache.
Video normálně ukazuje jeden snímek; všechna viditelná označená videa postupně
střídají nejvýše šest snímků, s předností souboru pod myší. Verze zdroje, změna
panelu i vymazání cache musí zneplatnit staré výsledky. Nastavení Previews umožní
náhledy vypnout, výslovně povolit síťová umístění a asynchronně vymazat cache.
Podrobnosti: [náhledy](2026-11-05-panel-views-thumbnails-design.md) a
[worker SDK](../../developer/WORKER_SDK.md).

Seznam všech knihoven a skutečně připnutých licencí je v
[licenčním přehledu](../../THIRD_PARTY.md). Distribuce přikládá příslušné copyright
a licenční texty; volitelný FFmpeg také odpovídající zdroj a konfiguraci sestavení.

## 2026-11-07: prohlížeče, barvy a navigace

[Aktuální rozšíření](2026-11-07-viewer-images-file-colors-design.md) doplňuje
zalamování Text/Source pomocí Ctrl+W, ověřené automatické binární zobrazení se
sloupci offset/hex/znaky a hledáním a samostatný obrazový plugin. Obrazy používají
stejné knihovny `image 0.25.10` a `moxcms 0.8.1` jako náhledy. Izolovaný proces
drží dekódovaný originál; host zprostředkuje čtení a dostává pouze omezený výřez.
Fit, 1:1, přiblížení kolem kurzoru a tažení fungují bez I/O na UI vlákně.
Public API/SDK, testy a plugin zůstávají samostatnými Cargo projekty; viz
[API obrazového vieweru](../../developer/IMAGE_VIEWER_API.md).

Options → Colors obsahuje pořadí pravidel masek/regexu, jednu základní barvu
písma a volitelné pozadí. Stav focus/selection se odvozuje automaticky s kontrolou
kontrastu; čtyři stavy jsou vidět v náhledu. Vyhodnocení probíhá na backendu proti
neměnnému snapshotu. Ctrl+F12 vedle masek a regexu nabízí Expression nad jménem,
velikostí běžného souboru, časem změny, druhem a hlášenými nativními atributy.
Výrazy používají `not`/`and`/`or`, typované hodnoty a výslovné UTC nebo číselný
časový posun. Chybějící metadata zůstávají neznámá i po negaci; jejich zahrnutí
určují `missing(...)`/`present(...)`. Neznámé kódování archivního jména se nenahrazuje
zobrazeným popiskem. Linux dot-jméno nepředstírá nativní příznak hidden.
Backend kontroluje 4096 UTF-8 bajtů, 512 tokenů, 256 uzlů, zanoření 32 a nejvýše 16
jmenných vzorů; neprovádí I/O, skripty ani čtení obsahu. Chybný či zastaralý návrh
nezmění platný filtr; dialog zachová text a ukáže řádek/sloupec chyby. Verze
nastavení 18 zachovává dřívější příkazy i volby kontrolních součtů. Kontext předků v inline stromu
čeká na TREE-01. [Syntaxe a limity](../../developer/METADATA_FILTERS.md),
[ověření Windows/Linux](../../verification/2026-11-07-metadata-expression-filters.md).
Carbon Green je deváté téma,
přidané bez změny identit předchozích témat.

Nabídka disků má neutrální tlačítko, využívá dostupnou výšku okna a zachovává
označení aktuálního disku v nabídce i mezi tlačítky. Oblíbené cesty jsou pouze
v nabídkách s čísly 1–0. Tlačítko Hot Paths je v pravé části horní lišty každého
panelu vedle voleb zobrazení a řazení. OneDrive má jeden primární zástupce na liště, zatímco
různé účty mají rozlišené položky v nabídce; stejné fyzické cíle se neduplikují.
Ikony zvláštních složek jsou větší v nezměněných tlačítkách. Archivní cesty mají
nativní oddělovače bez vložených mezer. Generickou ikonu listu u podporovaného
archivu nahrazuje vlastní SVG; konkrétní systémové ikony mají přednost.
[Původ prostředků](../../ASSET_PROVENANCE.md) je zdokumentovaný samostatně.

## 2026-11-07: rychlejší výpis, extrakce, PDF a porovnávání

[Navazující návrh](2026-11-07-archive-copy-pdf-comparison-design.md) specifikuje
nové hranice pluginů a kopírování. Windows výpis využívá metadata vrácená již
enumerací; další dotazy zůstávají pro providery, které metadata nedodaly. Výpis
nikdy nenahrazuje ověření identity při pozdější operaci. Místní měření je v
[ověření výkonu](../../verification/2026-11-07-listing-performance.md).

ZIP načítá seznam bez rozbalení všech souborů. Člen se dekóduje a ověří až při
čtení. Přibyly XZ, Bzip2 a Zstandard a odpovídající TAR varianty; GZIP/TAR/solid
7z mají nadále omezení sekvenčního zpracování. F5 zachytí provider, položky,
výběr a cílovou složku. Backend odmítne únik cestou, odkazy, cykly a změněné
zdroje; nativní cíl vzniká přes dočasný soubor a nezávislou SHA-256 kontrolu.
Existující cíle se nepřepisují bez implementované bezpečné politiky konfliktů.

Samostatný PDF plugin používá bezplatný open-source Hayro. Veřejné raster API
0.2 přidává seznam stránek a vykreslení konkrétní stránky při zachování
obrazového protokolu. Stránky přepínají tlačítka nebo Ctrl+PageUp/Down; zoom,
posuv, Escape a Space navazují na stávající viewer. PDF nespouští skripty ani
externí odkazy; nekompatibilní dokument skončí s chybou, nikoli tichým úspěchem.

Ctrl+Shift+C porovná dva vybrané nativní soubory, jinak jeden vybraný/zaměřený
soubor z každého panelu. Text používá `similar` s limitem vstupu a času; obrázky
sdílejí izolovaný dekodér se zobrazením Side-by-Side, Swipe a Onion Skin.
Výpočet i rasterizace běží mimo UI. Nejednoznačný výběr se odmítne. PDF se
nevydává za jednosnímkový obrázek k porovnání celého dokumentu.

Výchozí barevná pravidla rozlišují zdrojový kód, spustitelné soubory, obrázky,
videa a dokumenty. Uživatelská pravidla zůstávají zachována a výslovné odstranění
všech pravidel se ukládá. Ikony na liště disků mají 20 px pro myš a 26 px pro
dotyk, se stejnou výškou a šířkou tlačítek. [Vlastní příkazy](2026-11-07-user-commands-design.md)
jsou zatím návrh podle Salamander User Menu: lišta nad příkazovým řádkem,
uživatelské ikony/popisky, pracovní složka a přímý vstup do příslušného nastavení.

## 2026-11-07: okna, kompaktní rozhraní a ochrana navigačních položek

Minimální velikost hlavního okna je **320 × 200 logických pixelů**, nezávisle na
hustotě a zvoleném písmu. Při nedostatku místa se zobrazí aktivní panel a úzký
přepínač druhého panelu; příkazový řádek, Options a Jobs zůstávají dostupné.
Funkční tlačítka lze vodorovně posouvat. Po zvětšení se vrátí běžné dva panely.
Nepřiřazené Ctrl+F1–F12 mají viditelný popisek a jsou zakázané. Kliknutí zachytí
akci i panel při skutečném stisku, ještě před případným odložením události ve
Slint Flickable; uvolnění Ctrl během kliknutí nesmí změnit význam operace.

Položka `..` slouží pouze navigaci. Kopírování, mazání, přejmenování, editace
atributů a drag-and-drop ji nikdy nepoužijí jako zdroj. Skutečně označené položky
lze kopírovat nebo smazat i s kurzorem na `..`; přejmenování a viewer vyžadují
skutečnou zaměřenou položku. Vytváření souboru/složky a vložení do aktuálního
umístění zůstávají dostupné. UI používá identity položek a backend vlastní
validaci; text názvu souboru není bezpečnostní identita.

Dialogy mají tažitelné záhlaví a polohu omezenou rozměry okna. Zmenšení nesmí
znepřístupnit zavření ani ztratit rozepsané hodnoty. Editor atributů, času a
velikosti písmen má vlastní SVG ikonu dokumentu s hodinami; nadbytečný spodní
seznam náhledu souborů byl odstraněn. Validace požadovaných změn a následné
potvrzení přes připravené identity zůstávají na backendu.

Nastavení v15 ukládá fyzickou pozici vnějšího rámu, fyzickou velikost klientské
oblasti, normální rozměry před maximalizací a identifikaci monitoru. Změny se
ukládají asynchronně po 600 ms klidu, nejpozději po 2 s souvislé změny a při
ukončení. Dokud nejsou načtena uložená nastavení, nové výchozí rozměry je nesmějí
přepsat. Odpojený monitor znamená přesun do pracovní oblasti primárního monitoru.
Viewery a porovnávač přebírají fyzické rozměry a pozici hlavního okna; Escape je
vždy zavře i při změně uživatelského mapování. Omezení Waylandu a ověření jsou v
[dokumentu geometrie oken](../../verification/window-geometry.md).

Výběr obdélníkem začíná v prázdném místě panelu. Details, Brief a Thumbnails
používají dávky nesouvislých rozsahů vůči zachycené revizi seznamu a výběru;
podporují Ctrl/Shift, archivy a posun při přiblížení k okraji. Změna řazení,
filtru nebo složky zneplatní staré gesto. Tažení skutečné položky zachovává
souborový drag-and-drop. Aktuální implementaci a ověření popisuje
[výběrový kontrakt](../../developer/SELECTION.md).

### Spuštění pluginů ze sdíleného Windows buildu

Build helper vytvoří soukromou instalaci a registraci svázanou se SHA-256 přesného
host EXE. Spuštění `_build/release/me.exe` pak může použít bezpečné pluginy této
instalace, i když samotný checkout leží na sdíleném disku. Stejný backendový
resolver používají viewery, náhledy, archivy i obrazový porovnávač. Registrace ani
její cíl nesmějí být přepsatelné jiným účtem nebo vést přes junction. Kontroly
pluginů zůstávají aktivní a při startu se žádný plugin automaticky nekopíruje.
Soukromé přenosné instalace nadále používají sousední složky. Viz
[návrh](2026-11-07-trusted-runtime-resolution.md) a
[vývojářský kontrakt](../../developer/RUNTIME_DISCOVERY.md).

### Samostatné hledání souborů

Alt+F7 nebo lupa v panelu otevře samostatné posuvné okno hledání. Nejvýše čtyři
okna uchovávají nezávislé kořeny, dotaz a výsledky; panely lze dál používat.
Masky podporují více vzorů i vyloučení, výchozí `*.*` zahrnuje i názvy bez přípony.
Rekurze je zapnutá, rozlišování velikosti písmen volitelné. Obsah se hledá jako
doslovný text v UTF-8, případně s BOM, průběžně na backendu bez načtení celého
souboru do paměti. Nepodporovaná kódování a omezení jsou viditelná v souhrnu.

Výsledky přicházejí v omezených dávkách; Stop ponechá dosavadní výsledky a Escape
zavře konkrétní hledání. Enter, dvojklik nebo Go to file požádá backend o kontrolu
identity a revize výsledku a potom otevře nadřazenou složku v původním panelu.
Soubor se touto akcí nespouští. Nativní cesty se zachovávají odděleně od textu UI.
Otevření, editace či mazání z výsledků vyžaduje samostatný bezpečný backendový
kontrakt. Rozsah a limity: [Find API](../../developer/FIND.md).
